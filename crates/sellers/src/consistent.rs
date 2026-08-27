//! Rung 7, `consistent`: does the catalog's claim match what the endpoint
//! actually quotes?
//!
//! METHODOLOGY §10.3. Every other rung asks the seller a question. This one
//! asks whether two *other* parties agree about the seller — the catalog
//! that lists it and the endpoint that answers — and it is the only rung
//! that needs no request of its own, because both sides were already
//! observed and stored.
//!
//! # Why this is a finding and not a scolding
//!
//! A price that moved between a catalog snapshot and a probe is ordinary:
//! the seller changed it, the catalog has not re-crawled, and nobody did
//! anything wrong. What makes the comparison worth publishing is the
//! POPULATION rate — how much of the machine-readable economy an agent
//! cannot take at face value — and the per-field detail, so "the catalogs
//! are stale about price" and "the catalogs name the wrong payee" stay
//! different findings.
//!
//! So this module reports **which fields diverged**, never a verdict about
//! whose fault it is. `analysis/celo.md`'s rule holds: state what the
//! evidence shows, never intent.
//!
//! # The comparison is per resource, and one-sided on purpose
//!
//! A catalog claim is checked against the quote for THE SAME resource. A
//! seller whose endpoint offers payment options the catalog never mentioned
//! is not inconsistent — a catalog is allowed to be a subset — but a
//! catalog claim with no matching option at the endpoint is exactly the
//! divergence a buyer would hit.

use serde::{Deserialize, Serialize};

use crate::SellerStatus;
use crate::quote::Requirement;
use crate::reachable::Answer;

/// What a catalog said one resource costs. The subset of a payment
/// requirement a catalog actually publishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub resource: String,
    pub pay_to: String,
    pub network: String,
    /// Atomic units, as the catalog wrote them. `None` when the catalog
    /// listed the resource without a price — which is itself a fact, and not
    /// the same as claiming zero.
    ///
    /// Crosses JSON as a decimal string: a uint256 does not fit a JSON
    /// number, and `serde_json` panics rather than rounding. See
    /// [`crate::u128_str`].
    #[serde(default, with = "crate::u128_str::option")]
    pub amount: Option<u128>,
    pub asset: Option<String>,
}

/// One field the two sides disagree about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Divergence {
    /// The catalog named a price the endpoint does not offer for this payee.
    Amount { claimed: u128, quoted: u128 },
    /// Different token.
    Asset { claimed: String, quoted: String },
    /// Different chain.
    Network { claimed: String, quoted: String },
    /// The catalog claimed a price but the endpoint quoted nothing this
    /// claim could be matched against — no requirement for this payee.
    NoMatchingRequirement,
    /// The catalog listed the resource with no price at all, so there is
    /// nothing to compare. Not a divergence in the seller's favour or
    /// against it — an absence, reported as one.
    NothingClaimed,
}

impl Divergence {
    /// The word that goes in the `reason` column, so divergences can be
    /// counted by kind rather than read.
    pub fn field(&self) -> &'static str {
        match self {
            Self::Amount { .. } => "amount",
            Self::Asset { .. } => "asset",
            Self::Network { .. } => "network",
            Self::NoMatchingRequirement => "no_matching_requirement",
            Self::NothingClaimed => "nothing_claimed",
        }
    }
}

/// Rung 7's answer plus every field that diverged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsistencyVerdict {
    pub answer: Answer,
    pub divergences: Vec<Divergence>,
}

/// Compare one catalog claim against the requirements the endpoint quoted
/// for the same resource.
///
/// `quoted` is what `quote::judge` accepted for THIS seller — so payee
/// equality is already established, and what remains is price, token and
/// chain.
pub fn judge(claim: &Claim, quoted: &[Requirement]) -> ConsistencyVerdict {
    // Nothing quoted: rung 3 did not pass, so there is nothing to be
    // consistent WITH. `skipped`, never `fail` — the seller is not
    // inconsistent because we have only one side.
    if quoted.is_empty() {
        return ConsistencyVerdict {
            answer: Answer {
                status: SellerStatus::Skipped,
                reason: Some("rung_3_not_pass".into()),
            },
            divergences: Vec::new(),
        };
    }

    let (Some(claimed_amount), Some(claimed_asset)) = (claim.amount, claim.asset.as_ref()) else {
        return ConsistencyVerdict {
            answer: Answer {
                status: SellerStatus::Unprobed,
                reason: Some("nothing_claimed".into()),
            },
            divergences: vec![Divergence::NothingClaimed],
        };
    };

    // A catalog is allowed to be a SUBSET of what the endpoint offers, so
    // the question is whether any quoted requirement matches the claim —
    // not whether every quoted requirement does.
    let exact = quoted.iter().any(|r| {
        r.max_amount_required == claimed_amount
            && r.asset == *claimed_asset
            && crate::network::same(&r.network, &claim.network)
    });
    if exact {
        return ConsistencyVerdict {
            answer: Answer {
                status: SellerStatus::Pass,
                reason: None,
            },
            divergences: Vec::new(),
        };
    }

    // No exact match. Report which fields the CLOSEST requirement differs
    // in — closest by asset and network first, because a price change on
    // the same token is a different finding from a token change.
    let nearest = quoted
        .iter()
        .max_by_key(|r| {
            let asset_match = (r.asset == *claimed_asset) as u8;
            let network_match = crate::network::same(&r.network, &claim.network) as u8;
            asset_match * 2 + network_match
        })
        .expect("non-empty checked above");

    let mut divergences = Vec::new();
    if !crate::network::same(&nearest.network, &claim.network) {
        divergences.push(Divergence::Network {
            claimed: claim.network.clone(),
            quoted: nearest.network.clone(),
        });
    }
    if nearest.asset != *claimed_asset {
        divergences.push(Divergence::Asset {
            claimed: claimed_asset.clone(),
            quoted: nearest.asset.clone(),
        });
    }
    if nearest.max_amount_required != claimed_amount {
        divergences.push(Divergence::Amount {
            claimed: claimed_amount,
            quoted: nearest.max_amount_required,
        });
    }
    if divergences.is_empty() {
        // Nothing differs field by field, yet no requirement matched
        // wholesale — the claim's combination is not on offer.
        divergences.push(Divergence::NoMatchingRequirement);
    }

    let reason = divergences
        .iter()
        .map(Divergence::field)
        .collect::<Vec<_>>()
        .join("+");
    ConsistencyVerdict {
        answer: Answer {
            status: SellerStatus::Fail,
            reason: Some(reason),
        },
        divergences,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USDC: &str = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
    const OTHER: &str = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

    fn claim(amount: Option<u128>, asset: Option<&str>, network: &str) -> Claim {
        Claim {
            resource: "https://a.example/x".into(),
            pay_to: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            network: network.into(),
            amount,
            asset: asset.map(str::to_string),
        }
    }

    fn requirement(amount: u128, asset: &str, network: &str) -> Requirement {
        Requirement {
            scheme: "exact".into(),
            network: network.into(),
            max_amount_required: amount,
            asset: asset.into(),
            pay_to: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            resource: None,
        }
    }

    #[test]
    fn a_catalog_that_matches_the_endpoint_is_consistent() {
        let v = judge(
            &claim(Some(3000), Some(USDC), "eip155:8453"),
            &[requirement(3000, USDC, "eip155:8453")],
        );
        assert_eq!(v.answer.status, SellerStatus::Pass);
        assert!(v.divergences.is_empty());
    }

    #[test]
    fn the_two_conventions_for_one_network_are_still_consistent() {
        // A catalog writing `base` and an endpoint writing `eip155:8453`
        // agree. Comparing raw strings would manufacture a divergence for
        // every seller in the Bazaar.
        let v = judge(
            &claim(Some(3000), Some(USDC), "base"),
            &[requirement(3000, USDC, "eip155:8453")],
        );
        assert_eq!(v.answer.status, SellerStatus::Pass);
    }

    #[test]
    fn a_price_that_moved_diverges_on_amount_alone() {
        let v = judge(
            &claim(Some(3000), Some(USDC), "eip155:8453"),
            &[requirement(5000, USDC, "eip155:8453")],
        );
        assert_eq!(v.answer.status, SellerStatus::Fail);
        assert_eq!(v.answer.reason.as_deref(), Some("amount"));
        assert_eq!(
            v.divergences,
            vec![Divergence::Amount {
                claimed: 3000,
                quoted: 5000
            }]
        );
    }

    #[test]
    fn a_different_token_is_a_different_finding_from_a_different_price() {
        // "The catalogs are stale about price" and "the catalogs name the
        // wrong token" must not collapse into one number.
        let v = judge(
            &claim(Some(3000), Some(USDC), "eip155:8453"),
            &[requirement(3000, OTHER, "eip155:8453")],
        );
        assert_eq!(v.answer.reason.as_deref(), Some("asset"));
    }

    #[test]
    fn several_fields_diverging_are_all_reported() {
        let v = judge(
            &claim(Some(3000), Some(USDC), "eip155:8453"),
            &[requirement(9999, OTHER, "eip155:56")],
        );
        assert_eq!(v.answer.status, SellerStatus::Fail);
        assert_eq!(v.answer.reason.as_deref(), Some("network+asset+amount"));
        assert_eq!(v.divergences.len(), 3);
    }

    #[test]
    fn a_catalog_may_be_a_subset_of_what_the_endpoint_offers() {
        // The endpoint accepting more ways to pay than the catalog lists is
        // not an inconsistency; the claim is satisfied by one of them.
        let v = judge(
            &claim(Some(3000), Some(USDC), "eip155:8453"),
            &[
                requirement(9999, OTHER, "eip155:56"),
                requirement(3000, USDC, "eip155:8453"),
            ],
        );
        assert_eq!(v.answer.status, SellerStatus::Pass);
    }

    #[test]
    fn no_quote_means_nothing_to_compare_and_never_the_sellers_fail() {
        let v = judge(&claim(Some(3000), Some(USDC), "eip155:8453"), &[]);
        assert_eq!(v.answer.status, SellerStatus::Skipped);
        assert_eq!(v.answer.reason.as_deref(), Some("rung_3_not_pass"));
    }

    #[test]
    fn a_catalog_listing_with_no_price_is_an_absence_not_a_divergence() {
        // Listed without a price is a fact about the catalog, and it is not
        // the same as claiming zero.
        let v = judge(
            &claim(None, None, "eip155:8453"),
            &[requirement(1, USDC, "eip155:8453")],
        );
        assert_eq!(v.answer.status, SellerStatus::Unprobed);
        assert_eq!(v.answer.reason.as_deref(), Some("nothing_claimed"));
        assert!(!v.answer.status.is_about_the_seller());
    }
}

#[cfg(test)]
mod serialization_tests {
    use super::*;
    use crate::quote::Requirement;

    /// A price is a uint256. JSON numbers are not, and `serde_json` refuses
    /// any integer above `u64::MAX` with "number out of range" — which is a
    /// PANIC at the `json!` that builds an evidence row, not a rejected
    /// value. The first full sweep died on it after crawling all 148 pages:
    /// somewhere among 33,254 listings, one names a price larger than 2^64.
    ///
    /// So amounts serialize as decimal STRINGS, exactly as
    /// `payments.value_raw` is a NUMERIC-shaped string for the same reason:
    /// a price is never narrowed to fit a format.
    #[test]
    fn a_price_larger_than_u64_survives_serialization() {
        let huge = u128::from(u64::MAX) + 1;
        let claim = Claim {
            resource: "https://a.example/x".into(),
            pay_to: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            network: "eip155:8453".into(),
            amount: Some(huge),
            asset: Some("0x833589fcd6edb6e08f4c7c32d4f71b54bda02913".into()),
        };
        let json = serde_json::to_value(&claim).expect("a uint256 price must serialize");
        assert_eq!(json["amount"], serde_json::json!(huge.to_string()));

        // ...and comes back exactly, because a price that survives a round
        // trip approximately is a different price.
        let back: Claim = serde_json::from_value(json).unwrap();
        assert_eq!(back.amount, Some(huge));
    }

    #[test]
    fn a_quoted_requirement_survives_the_same_way() {
        let huge = u128::MAX;
        let req = Requirement {
            scheme: "exact".into(),
            network: "eip155:8453".into(),
            max_amount_required: huge,
            asset: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913".into(),
            pay_to: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            resource: None,
        };
        let json = serde_json::to_value(&req).expect("a uint256 quote must serialize");
        assert_eq!(
            json["max_amount_required"],
            serde_json::json!(huge.to_string())
        );
        let back: Requirement = serde_json::from_value(json).unwrap();
        assert_eq!(back.max_amount_required, huge);
    }

    #[test]
    fn an_amount_stored_as_a_json_number_still_reads_back() {
        // Rows written before this change carry numbers; they must not
        // become unreadable.
        let json = serde_json::json!({
            "resource": "https://a.example/x",
            "pay_to": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "network": "eip155:8453",
            "amount": 3000,
            "asset": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
        });
        let claim: Claim = serde_json::from_value(json).unwrap();
        assert_eq!(claim.amount, Some(3000));
    }
}
