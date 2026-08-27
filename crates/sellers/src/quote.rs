//! Rung 3, `quotes`: does a resource return a spec-valid 402?
//!
//! METHODOLOGY §10.3. The question is narrow on purpose — **can a buyer act
//! on this?** A 402 that omits the amount, or names an asset nobody can
//! identify, or quotes a payment address other than the one this seller is
//! defined by, does not tell a buyer how to pay this seller, whatever else
//! it contains.
//!
//! The judgment is `pass`/`fail` about the RESPONSE, never about the
//! operator. A host that declines the probe outright is `refused` and a
//! probe of ours that fell over is `error`; neither is decided here, because
//! neither is a fact about the document (§10.3, and §4's vocabulary).
//!
//! Field names follow the x402 payment-requirements shape, pinned to a spec
//! commit exactly as rung 4 of the registration census pins ERC-8004. When
//! that pin moves, this parser and the pin move together in one changelog
//! entry — a parser that quietly accepts two spec versions is two methods.

use serde::{Deserialize, Serialize};

use crate::identity::{Network, SellerId, normalize_pay_to};

/// One payment requirement a 402 offered — the subset METHODOLOGY §10.3
/// requires, normalized. Fields the spec carries but this census does not
/// judge (`description`, `mimeType`, `maxTimeoutSeconds`, `extra`) are
/// deliberately not modelled: a rung that reads a field must say what it
/// concludes from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    pub scheme: String,
    pub network: String,
    /// Atomic units of `asset`, as quoted. Kept exact — a price is not a
    /// float, and rounding one to buy it would be this census paying a
    /// number it did not read. Crosses JSON as a decimal string for the
    /// same reason; see [`crate::u128_str`].
    #[serde(with = "crate::u128_str")]
    pub max_amount_required: u128,
    /// The asset contract, normalized like any other address.
    pub asset: String,
    /// The payee, normalized. A requirement is only this seller's if this
    /// equals its `pay_to`.
    pub pay_to: String,
    pub resource: Option<String>,
}

/// Why a 402 body did not amount to a quote. Every variant is recorded on
/// the row verbatim, so "how many sellers quote badly, and how" is a
/// countable question rather than an impression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Malformed {
    /// The body was not JSON at all.
    NotJson,
    /// No `accepts` array — the one field the shape is built around.
    NoAccepts,
    /// `accepts` was present and empty: a 402 that offers no way to pay.
    NoRequirements,
    /// Every entry was missing a required field or unparseable. Carries the
    /// first field that was missing, for the evidence row.
    IncompleteRequirement { field: String },
    /// Entries parsed, but none named this seller's `pay_to`. The endpoint
    /// quoted somebody else — which is a fact about this seller's listing,
    /// not about the endpoint's health.
    NotThisSellersPayTo,
}

/// Rung 3's answer for one resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuoteVerdict {
    /// `pass` — at least one spec-valid requirement naming this seller.
    Quotes(Vec<Requirement>),
    /// `fail`, with the reason on the row.
    Fail(Malformed),
}

/// Judge one 402 body for one seller.
///
/// `body` is the raw response bytes as text; `network` is the encoding rule
/// for addresses (§10.1), not a filter — a quote naming another chain still
/// parses, and the chain it names is recorded rather than judged, because
/// "sells on a chain we do not sweep" is coverage, not failure.
pub fn judge(seller: &SellerId, body: &str, network: Network) -> QuoteVerdict {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return QuoteVerdict::Fail(Malformed::NotJson);
    };
    let Some(accepts) = value.get("accepts").and_then(|a| a.as_array()) else {
        return QuoteVerdict::Fail(Malformed::NoAccepts);
    };
    if accepts.is_empty() {
        return QuoteVerdict::Fail(Malformed::NoRequirements);
    }

    let mut missing_field: Option<String> = None;
    let mut mine = Vec::new();
    for entry in accepts {
        match parse_requirement(entry, network) {
            Ok(req) => {
                if req.pay_to == seller.pay_to {
                    mine.push(req);
                }
            }
            Err(field) => {
                missing_field.get_or_insert(field);
            }
        }
    }

    if !mine.is_empty() {
        return QuoteVerdict::Quotes(mine);
    }
    match missing_field {
        // Nothing this seller could be paid by, and at least one entry was
        // structurally incomplete: the incompleteness is the better answer,
        // because it is the one the operator can act on.
        Some(field) => QuoteVerdict::Fail(Malformed::IncompleteRequirement { field }),
        None => QuoteVerdict::Fail(Malformed::NotThisSellersPayTo),
    }
}

/// One `accepts[]` entry, or the name of the first field that made it
/// unusable.
fn parse_requirement(entry: &serde_json::Value, network: Network) -> Result<Requirement, String> {
    let text = |key: &str| -> Result<String, String> {
        entry
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| key.to_string())
    };
    let scheme = text("scheme")?;
    let net = text("network")?;

    // Amounts are quoted as decimal STRINGS in the x402 shape, because a
    // uint256 does not survive JSON's number type. A number is accepted too
    // and read exactly — but only if it is an integer, since a fractional
    // atomic unit is not a price anybody can pay.
    //
    // Two field names, both live: `maxAmountRequired` is x402 v1's, and
    // `amount` is what v2 bodies and the Bazaar's own listings carry
    // (observed 2026-08-20). Reading only the documented one would price
    // nothing on a v2 endpoint.
    let amount_value = entry
        .get("maxAmountRequired")
        .or_else(|| entry.get("amount"))
        .ok_or_else(|| "maxAmountRequired".to_string())?;
    let max_amount_required = match amount_value {
        serde_json::Value::String(s) => s
            .trim()
            .parse::<u128>()
            .map_err(|_| "maxAmountRequired".to_string())?,
        serde_json::Value::Number(n) => {
            n.as_u128().ok_or_else(|| "maxAmountRequired".to_string())?
        }
        _ => return Err("maxAmountRequired".to_string()),
    };

    let asset = normalize_pay_to(&text("asset")?, network).map_err(|_| "asset".to_string())?;
    let pay_to = normalize_pay_to(&text("payTo")?, network).map_err(|_| "payTo".to_string())?;

    Ok(Requirement {
        scheme,
        network: net,
        max_amount_required,
        asset,
        pay_to,
        resource: entry
            .get("resource")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seller() -> SellerId {
        SellerId::new(
            "0xAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAa",
            "https://api.example.com/weather",
            Network::Evm,
        )
        .unwrap()
    }

    fn body_with(pay_to: &str) -> String {
        format!(
            r#"{{"x402Version":1,"error":"payment required","accepts":[{{
                "scheme":"exact","network":"base","maxAmountRequired":"1000",
                "asset":"0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
                "payTo":"{pay_to}","resource":"https://api.example.com/weather"}}]}}"#
        )
    }

    #[test]
    fn a_spec_valid_402_naming_this_seller_quotes() {
        let v = judge(
            &seller(),
            &body_with("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            Network::Evm,
        );
        let QuoteVerdict::Quotes(reqs) = v else {
            panic!("expected a quote, got {v:?}");
        };
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].max_amount_required, 1000);
        assert_eq!(reqs[0].scheme, "exact");
    }

    #[test]
    fn the_payee_is_matched_after_normalization_not_as_written() {
        // The 402 writes the address checksummed; the seller id is
        // lowercased. Same 20 bytes, so this must match — else every
        // checksum-writing seller would read as quoting somebody else.
        let v = judge(
            &seller(),
            &body_with("0xAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAa"),
            Network::Evm,
        );
        assert!(matches!(v, QuoteVerdict::Quotes(_)), "got {v:?}");
    }

    #[test]
    fn a_402_quoting_somebody_else_is_not_this_sellers_quote() {
        let v = judge(
            &seller(),
            &body_with("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            Network::Evm,
        );
        assert_eq!(v, QuoteVerdict::Fail(Malformed::NotThisSellersPayTo));
    }

    #[test]
    fn a_quote_with_no_amount_cannot_be_acted_on() {
        let body = r#"{"accepts":[{"scheme":"exact","network":"base",
            "asset":"0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
            "payTo":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#;
        assert_eq!(
            judge(&seller(), body, Network::Evm),
            QuoteVerdict::Fail(Malformed::IncompleteRequirement {
                field: "maxAmountRequired".into()
            })
        );
    }

    #[test]
    fn the_shapes_that_are_not_quotes_at_all_are_told_apart() {
        assert_eq!(
            judge(&seller(), "<html>402</html>", Network::Evm),
            QuoteVerdict::Fail(Malformed::NotJson)
        );
        assert_eq!(
            judge(
                &seller(),
                r#"{"x402Version":1,"error":"nope"}"#,
                Network::Evm
            ),
            QuoteVerdict::Fail(Malformed::NoAccepts)
        );
        assert_eq!(
            judge(&seller(), r#"{"accepts":[]}"#, Network::Evm),
            QuoteVerdict::Fail(Malformed::NoRequirements)
        );
    }

    #[test]
    fn one_valid_requirement_among_broken_ones_is_still_a_quote() {
        // A buyer can act on it, which is the whole question.
        let body = r#"{"accepts":[
            {"scheme":"exact","network":"base"},
            {"scheme":"exact","network":"base","maxAmountRequired":"500",
             "asset":"0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
             "payTo":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#;
        let QuoteVerdict::Quotes(reqs) = judge(&seller(), body, Network::Evm) else {
            panic!("a usable requirement was present");
        };
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].max_amount_required, 500);
    }

    #[test]
    fn an_amount_quoted_as_a_json_number_is_read_exactly() {
        let body = r#"{"accepts":[{"scheme":"exact","network":"base","maxAmountRequired":1000,
            "asset":"0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
            "payTo":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#;
        let QuoteVerdict::Quotes(reqs) = judge(&seller(), body, Network::Evm) else {
            panic!("expected a quote");
        };
        assert_eq!(reqs[0].max_amount_required, 1000);
    }

    #[test]
    fn a_fractional_atomic_amount_is_not_a_price() {
        let body = r#"{"accepts":[{"scheme":"exact","network":"base","maxAmountRequired":10.5,
            "asset":"0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
            "payTo":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#;
        assert_eq!(
            judge(&seller(), body, Network::Evm),
            QuoteVerdict::Fail(Malformed::IncompleteRequirement {
                field: "maxAmountRequired".into()
            })
        );
    }

    #[test]
    fn a_v2_quote_naming_the_amount_field_is_priced_too() {
        // x402 v2 (and the Bazaar's own listings) carry `amount` where v1
        // carried `maxAmountRequired`. Reading only the documented name
        // would price nothing on a v2 endpoint.
        let body = r#"{"x402Version":2,"accepts":[{"scheme":"exact","network":"eip155:8453",
            "amount":"3000","asset":"0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
            "payTo":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#;
        let QuoteVerdict::Quotes(reqs) = judge(&seller(), body, Network::Evm) else {
            panic!("a v2 quote is a quote");
        };
        assert_eq!(reqs[0].max_amount_required, 3000);
    }

    #[test]
    fn a_quote_naming_a_chain_we_do_not_sweep_still_parses() {
        // Coverage, not failure: what it named is recorded, and the decision
        // to buy is made elsewhere (§10.4).
        let body = r#"{"accepts":[{"scheme":"exact","network":"solana","maxAmountRequired":"100",
            "asset":"0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
            "payTo":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#;
        let QuoteVerdict::Quotes(reqs) = judge(&seller(), body, Network::Evm) else {
            panic!("expected a quote");
        };
        assert_eq!(reqs[0].network, "solana");
    }
}
