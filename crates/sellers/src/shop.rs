//! What this census is allowed to buy.
//!
//! METHODOLOGY §10.4. Rung 4 pays real sellers real money, which is only
//! defensible under rules written down before the first purchase — so they
//! are here, as functions, rather than in the shopper binary where they
//! would be settings somebody could nudge.
//!
//! Three of them live in this module:
//!
//! * **The cap.** $0.10 face value, and *face value* means an asset whose
//!   unit this census can actually read. A quote it cannot price is
//!   [`Unprobed::Unpriced`] — never bought "just to see", and never silently
//!   dropped from the denominator.
//! * **One purchase per seller per sweep**, the cheapest at-or-under-cap
//!   resource. [`decide`] takes every requirement a seller offered and
//!   returns at most one thing to buy.
//! * **Nothing outside the swept scope.** Sweep 1 is Base/USDC (§10.5); a
//!   seller quoting only elsewhere is [`Unprobed::OutOfScopeNetwork`], which
//!   is coverage this census does not have, stated as such.
//!
//! Every `unprobed` reason is published beside the delivery rate it
//! qualifies — "delivered 61% of the 83% under cap" is the honest sentence
//! shape, and it needs these counts to exist.

use serde::{Deserialize, Serialize};

use crate::quote::Requirement;

/// The price cap, in whole US cents of face value. Ten cents (§10.4).
pub const CAP_CENTS: u64 = 10;

/// Why a seller was not bought from. Recorded verbatim on the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unprobed {
    /// Every quote priced above the cap. We know what it costs and chose not
    /// to pay it.
    OverCap,
    /// Quoted in an asset this census cannot read at face value. Pricing it
    /// would mean adopting a conversion rule and a price source — a
    /// methodology invented to spend ten cents — so it is labelled instead.
    Unpriced,
    /// Quoted only on a network this sweep does not cover.
    OutOfScopeNetwork,
    /// The seller has no quote to buy from at all (rung 3 did not pass).
    NoQuote,
}

/// The shopper's decision for one seller, for one sweep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    /// Buy exactly this requirement — the index into the slice `decide` was
    /// given — at this face value.
    Buy { index: usize, cents: u64 },
    /// Buy nothing, and say why.
    Skip { reason: Unprobed },
}

/// One asset this census can read at face value: its contract, the network
/// it lives on, and its decimals.
///
/// An allowlist rather than a lookup, because "what is this token worth" is
/// exactly the question a price feed answers with a number that moves, and
/// this instrument's cap must mean the same thing in two different weeks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PricedAsset {
    pub network: &'static str,
    pub address: &'static str,
    pub decimals: u32,
}

/// What sweep 1 covers: Base, USDC (METHODOLOGY §10.5). Adding an entry
/// changes the population that can be probed and is a changelog event.
///
/// The network is written in its canonical CAIP-2 form because that is what
/// the catalogs actually serve — the Bazaar names `eip155:8453`, never
/// `base` — and comparisons go through [`crate::network`] so a quote saying
/// `base` matches it anyway.
pub const SWEEP_ONE_ASSETS: &[PricedAsset] = &[PricedAsset {
    network: crate::network::BASE,
    // Circle's canonical USDC on Base, lowercased like every other address
    // this crate handles.
    address: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
    decimals: 6,
}];

/// The face value of `amount` atomic units, in whole cents, ROUNDED UP —
/// or `None` when the asset is not one this census can read.
///
/// Rounding up is the direction that cannot overspend: a quote of 10.0001
/// cents reads as 11 and falls outside a ten-cent cap, where rounding down
/// would have this census pay a price it had decided was too high.
pub fn face_value_cents(
    assets: &[PricedAsset],
    network: &str,
    asset: &str,
    amount: u128,
) -> Option<u64> {
    let entry = assets
        .iter()
        .find(|a| crate::network::same(a.network, network) && a.address == asset)?;
    let scale = 10u128.checked_pow(entry.decimals)?;
    let cents = amount.checked_mul(100)?.div_ceil(scale);
    u64::try_from(cents).ok()
}

/// Decide what to buy from one seller this sweep, given every requirement it
/// quoted across its resources.
///
/// When nothing is buyable the reason is the most INFORMATIVE one available,
/// in this order: a price we read and declined (`OverCap`) says more than an
/// asset we could not read (`Unpriced`), which says more than a network we
/// do not sweep (`OutOfScopeNetwork`). A row therefore carries the strongest
/// thing this census actually learned about the seller.
pub fn decide(assets: &[PricedAsset], requirements: &[Requirement]) -> Decision {
    if requirements.is_empty() {
        return Decision::Skip {
            reason: Unprobed::NoQuote,
        };
    }

    let in_scope: Vec<usize> = requirements
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            assets
                .iter()
                .any(|a| crate::network::same(a.network, &r.network))
        })
        .map(|(i, _)| i)
        .collect();
    if in_scope.is_empty() {
        return Decision::Skip {
            reason: Unprobed::OutOfScopeNetwork,
        };
    }

    let mut cheapest: Option<(usize, u64)> = None;
    let mut saw_over_cap = false;
    let mut saw_unpriced = false;
    for i in in_scope {
        let r = &requirements[i];
        match face_value_cents(assets, &r.network, &r.asset, r.max_amount_required) {
            Some(cents) if cents <= CAP_CENTS => {
                // Cheapest wins; ties keep the earlier index so two runs over
                // the same quotes buy the same resource.
                if cheapest.is_none_or(|(_, best)| cents < best) {
                    cheapest = Some((i, cents));
                }
            }
            Some(_) => saw_over_cap = true,
            None => saw_unpriced = true,
        }
    }

    match cheapest {
        Some((index, cents)) => Decision::Buy { index, cents },
        None if saw_over_cap => Decision::Skip {
            reason: Unprobed::OverCap,
        },
        None if saw_unpriced => Decision::Skip {
            reason: Unprobed::Unpriced,
        },
        None => Decision::Skip {
            reason: Unprobed::OutOfScopeNetwork,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(network: &str, asset: &str, amount: u128) -> Requirement {
        Requirement {
            scheme: "exact".into(),
            network: network.into(),
            max_amount_required: amount,
            asset: asset.into(),
            pay_to: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            resource: None,
        }
    }

    const USDC: &str = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";

    #[test]
    fn exactly_ten_cents_is_at_the_cap_and_is_bought() {
        // 100,000 atomic units of a 6-decimal dollar is $0.10 exactly. The
        // cap is inclusive: "≤ $0.10" is what the method says.
        let d = decide(SWEEP_ONE_ASSETS, &[req("base", USDC, 100_000)]);
        assert_eq!(
            d,
            Decision::Buy {
                index: 0,
                cents: 10
            }
        );
    }

    #[test]
    fn one_atomic_unit_over_the_cap_is_not_bought() {
        // The rounding direction that cannot overspend.
        let d = decide(SWEEP_ONE_ASSETS, &[req("base", USDC, 100_001)]);
        assert_eq!(
            d,
            Decision::Skip {
                reason: Unprobed::OverCap
            }
        );
    }

    #[test]
    fn the_cheapest_at_or_under_cap_resource_is_the_one_bought() {
        let reqs = [
            req("base", USDC, 90_000),  // 9c
            req("base", USDC, 10_000),  // 1c — the one
            req("base", USDC, 500_000), // 50c, over cap
        ];
        assert_eq!(
            decide(SWEEP_ONE_ASSETS, &reqs),
            Decision::Buy { index: 1, cents: 1 }
        );
    }

    #[test]
    fn a_tie_keeps_the_earlier_quote_so_two_sweeps_buy_the_same_thing() {
        let reqs = [req("base", USDC, 10_000), req("base", USDC, 10_000)];
        assert_eq!(
            decide(SWEEP_ONE_ASSETS, &reqs),
            Decision::Buy { index: 0, cents: 1 }
        );
    }

    #[test]
    fn an_asset_we_cannot_read_is_labelled_never_guessed_at() {
        // Pricing this would mean adopting a conversion rule and a price
        // source to spend ten cents.
        let d = decide(
            SWEEP_ONE_ASSETS,
            &[req("base", "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef", 1)],
        );
        assert_eq!(
            d,
            Decision::Skip {
                reason: Unprobed::Unpriced
            }
        );
    }

    #[test]
    fn a_network_this_sweep_does_not_cover_is_stated_as_coverage() {
        let d = decide(SWEEP_ONE_ASSETS, &[req("solana", USDC, 1_000)]);
        assert_eq!(
            d,
            Decision::Skip {
                reason: Unprobed::OutOfScopeNetwork
            }
        );
    }

    #[test]
    fn no_quote_at_all_is_its_own_reason_not_a_price_judgment() {
        assert_eq!(
            decide(SWEEP_ONE_ASSETS, &[]),
            Decision::Skip {
                reason: Unprobed::NoQuote
            }
        );
    }

    #[test]
    fn the_most_informative_reason_wins_when_several_apply() {
        // A price we read and declined says more about this seller than an
        // asset we could not read.
        let reqs = [
            req("base", "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef", 1),
            req("base", USDC, 1_000_000), // $1.00
        ];
        assert_eq!(
            decide(SWEEP_ONE_ASSETS, &reqs),
            Decision::Skip {
                reason: Unprobed::OverCap
            }
        );
    }

    #[test]
    fn a_buyable_quote_beats_every_reason_to_skip() {
        let reqs = [
            req("solana", USDC, 1),
            req("base", "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef", 1),
            req("base", USDC, 50_000), // 5c
        ];
        assert_eq!(
            decide(SWEEP_ONE_ASSETS, &reqs),
            Decision::Buy { index: 2, cents: 5 }
        );
    }

    #[test]
    fn face_value_rounds_up_so_a_sub_cent_price_is_never_free() {
        // 1 atomic unit of USDC is $0.000001 — a real price, and one cent is
        // the smallest thing this cap can express. Rounding it to zero would
        // make "free" a category the census does not measure.
        assert_eq!(face_value_cents(SWEEP_ONE_ASSETS, "base", USDC, 1), Some(1));
        assert_eq!(face_value_cents(SWEEP_ONE_ASSETS, "base", USDC, 0), Some(0));
    }

    #[test]
    fn a_quote_naming_the_chain_in_caip2_is_in_scope() {
        // THE near miss, as a regression test. Every one of the Bazaar's
        // 15,155 resources names `eip155:8453`; the scope says Base. Match
        // the raw strings and the whole catalog reads as out of scope, and
        // the census publishes a delivery rate over nobody.
        let d = decide(SWEEP_ONE_ASSETS, &[req("eip155:8453", USDC, 50_000)]);
        assert_eq!(d, Decision::Buy { index: 0, cents: 5 });
    }

    #[test]
    fn the_network_name_matches_case_insensitively_but_the_asset_does_not() {
        // Network names are free text in the quote ("Base", "base"); asset
        // addresses arrive normalized from `quote::judge`, so a mismatch
        // there means a genuinely different contract.
        assert_eq!(
            face_value_cents(SWEEP_ONE_ASSETS, "Base", USDC, 100_000),
            Some(10)
        );
        assert_eq!(
            face_value_cents(
                SWEEP_ONE_ASSETS,
                "base",
                "0x833589FCD6EDB6E08F4C7C32D4F71B54BDA02913",
                100_000
            ),
            None
        );
    }
}
