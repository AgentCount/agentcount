//! Rungs 2 and 3, judged from ONE observation.
//!
//! METHODOLOGY §10.3. `reachable` asks whether the host answers at all, and
//! `quotes` asks whether what it answered is a payment quote a buyer could
//! act on. Both are answered by the same request, deliberately: two requests
//! to learn two things about the same URL would double this census's traffic
//! to every seller for no additional fact.
//!
//! # A 402 means the opposite thing here
//!
//! The registration census reads HTTP 402 as `refused` — an agent's document
//! behind a payment wall is a document we were declined
//! (`checks::refusal::declined_us`, which admits 401, 402, 407, 429, 503).
//!
//! For a SELLER, a 402 is the product. It is the seller working: the
//! protocol's way of saying "here is what this costs and where to pay it".
//! Carrying the agent census's list over unchanged would have booked every
//! correctly-functioning x402 seller as having declined us — the single
//! most inverted number this instrument could publish.
//!
//! So the decline list here is 401, 407, 429, 503: the same statuses **minus
//! the one this instrument exists to receive**. That difference is the whole
//! reason this module is not a call into `checks::refusal`.

use serde::{Deserialize, Serialize};

use crate::SellerStatus;
use crate::identity::{Network, SellerId};
use crate::quote::{self, Malformed, QuoteVerdict, Requirement};

/// What one probe of one resource actually observed. Facts only — no
/// judgement — produced by the I/O half and judged here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// The origin answered, with this status and (when read) this body.
    Response { status: u16, body: Option<String> },
    /// We were not permitted to ask: robots.txt disallowed us, or we could
    /// not establish permission at all (RFC 9309 §2.3.1.4). The origin
    /// declined through the one channel the web has for declining.
    NotPermitted { reason: String },
    /// OUR failure: a timeout, a TLS error, DNS, a body we could not read.
    /// Never the seller's.
    ProbeFailed { reason: String },
}

/// One rung's answer, with the word that goes in the `reason` column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub status: SellerStatus,
    pub reason: Option<String>,
}

impl Answer {
    fn new(status: SellerStatus, reason: Option<&str>) -> Self {
        Self {
            status,
            reason: reason.map(str::to_string),
        }
    }
}

/// Rungs 2 and 3 for one seller, plus the requirements a quote carried (so
/// the shopper can price them without re-fetching).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeVerdict {
    pub reachable: Answer,
    pub quotes: Answer,
    pub requirements: Vec<Requirement>,
}

/// The statuses that mean the origin is there and declined us.
///
/// `checks::refusal::declined_us` minus 402. See the module doc: for a
/// seller, 402 is the product, not a refusal.
pub fn declined_us(http_status: u16) -> bool {
    matches!(http_status, 401 | 407 | 429 | 503)
}

/// Judge one probe.
pub fn judge(seller: &SellerId, observed: &Observed, network: Network) -> ProbeVerdict {
    match observed {
        // Not permitted: rung 2 is `refused`, and rung 3 was never asked.
        // `skipped` rather than `fail` — a question we did not ask has no
        // answer, and a seller must never be recorded as failing to quote
        // because we chose not to look.
        Observed::NotPermitted { reason } => ProbeVerdict {
            reachable: Answer::new(SellerStatus::Refused, Some(reason)),
            quotes: Answer::new(SellerStatus::Skipped, Some("rung_2_not_pass")),
            requirements: Vec::new(),
        },
        // Our failure. Never the seller's, and never churn (§9's rule, which
        // this instrument inherits from day one).
        Observed::ProbeFailed { reason } => ProbeVerdict {
            reachable: Answer::new(SellerStatus::Error, Some(reason)),
            quotes: Answer::new(SellerStatus::Skipped, Some("rung_2_not_pass")),
            requirements: Vec::new(),
        },
        Observed::Response { status, body } => {
            if declined_us(*status) {
                return ProbeVerdict {
                    reachable: Answer::new(SellerStatus::Refused, Some("declined")),
                    quotes: Answer::new(SellerStatus::Skipped, Some("rung_2_not_pass")),
                    requirements: Vec::new(),
                };
            }

            // ANY other answer is reachability. The question is existence,
            // not health: a 404 or a 500 is a host that is there and talking,
            // and judging it otherwise would fold two different findings
            // ("nothing at this address" and "this address is broken") into
            // one word.
            let reachable = Answer::new(SellerStatus::Pass, None);

            if *status != 402 {
                // The seller answered without asking to be paid. Not a
                // quote — and the status is the reason, so "how many listed
                // resources just serve 200 to anyone" is countable.
                return ProbeVerdict {
                    reachable,
                    quotes: Answer::new(SellerStatus::Fail, Some(&format!("http_{status}"))),
                    requirements: Vec::new(),
                };
            }

            let Some(body) = body else {
                return ProbeVerdict {
                    reachable,
                    quotes: Answer::new(SellerStatus::Fail, Some("no_body")),
                    requirements: Vec::new(),
                };
            };
            match quote::judge(seller, body, network) {
                QuoteVerdict::Quotes(requirements) => ProbeVerdict {
                    reachable,
                    quotes: Answer::new(SellerStatus::Pass, None),
                    requirements,
                },
                QuoteVerdict::Fail(m) => ProbeVerdict {
                    reachable,
                    quotes: Answer::new(SellerStatus::Fail, Some(malformed_reason(&m))),
                    requirements: Vec::new(),
                },
            }
        }
    }
}

/// The `reason` word for a malformed quote — one spelling per shape, so the
/// column can be counted rather than read.
fn malformed_reason(m: &Malformed) -> &'static str {
    match m {
        Malformed::NotJson => "not_json",
        Malformed::NoAccepts => "no_accepts",
        Malformed::NoRequirements => "no_requirements",
        Malformed::IncompleteRequirement { .. } => "incomplete_requirement",
        Malformed::NotThisSellersPayTo => "not_this_sellers_pay_to",
    }
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

    fn quote_body() -> String {
        r#"{"x402Version":2,"accepts":[{"scheme":"exact","network":"eip155:8453",
            "amount":"3000","asset":"0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
            "payTo":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#
            .to_string()
    }

    #[test]
    fn a_402_is_the_product_not_a_refusal() {
        // THE inversion this module exists to prevent. The registration
        // census reads 402 as `refused`; carrying that over would have
        // booked every correctly-working seller as having declined us.
        assert!(!declined_us(402), "402 is what a working seller answers");
        let v = judge(
            &seller(),
            &Observed::Response {
                status: 402,
                body: Some(quote_body()),
            },
            Network::Evm,
        );
        assert_eq!(v.reachable.status, SellerStatus::Pass);
        assert_eq!(v.quotes.status, SellerStatus::Pass);
        assert_eq!(v.requirements.len(), 1);
    }

    #[test]
    fn the_other_four_decline_statuses_still_decline() {
        // Everything `checks::refusal` admits EXCEPT 402.
        for status in [401, 407, 429, 503] {
            assert!(declined_us(status), "{status} declines");
            let v = judge(
                &seller(),
                &Observed::Response { status, body: None },
                Network::Evm,
            );
            assert_eq!(v.reachable.status, SellerStatus::Refused, "{status}");
            // ...and rung 3 was never asked, so it is skipped, not failed.
            assert_eq!(v.quotes.status, SellerStatus::Skipped, "{status}");
        }
    }

    #[test]
    fn reachability_is_existence_not_health() {
        // A 404 and a 500 are hosts that are there and talking. Judging them
        // unreachable would fold "nothing at this address" and "this address
        // is broken" into one word.
        for status in [200, 404, 500] {
            let v = judge(
                &seller(),
                &Observed::Response { status, body: None },
                Network::Evm,
            );
            assert_eq!(v.reachable.status, SellerStatus::Pass, "{status}");
            assert_eq!(v.quotes.status, SellerStatus::Fail, "{status}");
            assert_eq!(
                v.quotes.reason.as_deref(),
                Some(&format!("http_{status}")[..])
            );
        }
    }

    #[test]
    fn our_own_failure_is_never_the_sellers() {
        let v = judge(
            &seller(),
            &Observed::ProbeFailed {
                reason: "timeout".into(),
            },
            Network::Evm,
        );
        assert_eq!(v.reachable.status, SellerStatus::Error);
        assert!(!v.reachable.status.is_about_the_seller());
        assert_eq!(v.quotes.status, SellerStatus::Skipped);
    }

    #[test]
    fn a_host_that_did_not_permit_the_ask_is_refused_and_never_failed() {
        let v = judge(
            &seller(),
            &Observed::NotPermitted {
                reason: "robots_disallowed".into(),
            },
            Network::Evm,
        );
        assert_eq!(v.reachable.status, SellerStatus::Refused);
        assert_eq!(v.reachable.reason.as_deref(), Some("robots_disallowed"));
        // The seller is not recorded as failing to quote because we chose
        // not to look.
        assert_eq!(v.quotes.status, SellerStatus::Skipped);
    }

    #[test]
    fn a_402_quoting_somebody_else_reaches_but_does_not_quote() {
        let body = r#"{"accepts":[{"scheme":"exact","network":"eip155:8453","amount":"1",
            "asset":"0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
            "payTo":"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}]}"#;
        let v = judge(
            &seller(),
            &Observed::Response {
                status: 402,
                body: Some(body.into()),
            },
            Network::Evm,
        );
        assert_eq!(v.reachable.status, SellerStatus::Pass);
        assert_eq!(v.quotes.status, SellerStatus::Fail);
        assert_eq!(v.quotes.reason.as_deref(), Some("not_this_sellers_pay_to"));
    }

    #[test]
    fn every_malformed_shape_has_one_countable_reason_word() {
        for (body, expected) in [
            ("<html/>", "not_json"),
            (r#"{"x402Version":2}"#, "no_accepts"),
            (r#"{"accepts":[]}"#, "no_requirements"),
            (
                r#"{"accepts":[{"scheme":"exact"}]}"#,
                "incomplete_requirement",
            ),
        ] {
            let v = judge(
                &seller(),
                &Observed::Response {
                    status: 402,
                    body: Some(body.into()),
                },
                Network::Evm,
            );
            assert_eq!(v.quotes.reason.as_deref(), Some(expected), "{body}");
            assert_eq!(v.quotes.status, SellerStatus::Fail);
        }
    }
}
