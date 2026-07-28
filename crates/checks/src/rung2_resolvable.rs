//! Rung 2 — `resolvable`: can the agent's declared registration document be
//! retrieved at all?
//!
//! `Fail` is reserved for facts about what the agent published: no `tokenURI`,
//! a scheme we don't support, or an HTTP status that means the document
//! itself is unavailable (402, 404, 500, ...). `Error` is reserved for OUR
//! limitations: a timeout, a DNS/TLS failure, robots.txt being unreachable,
//! or our IPFS gateway failing. Getting this backwards publishes a false
//! accusation about a real project, so every branch below is judged against
//! that line, not against "did we get a 200".
//!
//! Three decisions worth spelling out because they are easy to get wrong:
//!
//! * **HTTP 402 fails.** A payment challenge is not the registration
//!   document, and you cannot parse a document you never received. The
//!   status and (whatever) body are still archived by the prober; this rung
//!   only records that resolution failed and why.
//! * **`data:` URIs pass, unconditionally.** 15,495 agents (25.8% of the
//!   registry) publish their registration document inline, and the spec
//!   explicitly permits it (`ERC8004SPEC.md` L52). An inline URI is the
//!   strongest possible answer to "can this be retrieved" — the document is
//!   already in hand, no network round trip needed. Evidence for this case
//!   carries `inline: true` and the decoded byte count, and deliberately
//!   carries no HTTP fields (`request_url`, `final_url`, `http_status`,
//!   `elapsed_ms`) — none of them were ever populated, and printing them as
//!   `null` would look like a fetch was attempted when it wasn't.
//! * **`ipfs://` goes through one named gateway.** The evidence records which
//!   one via `via_gateway`, so a reader can distinguish "this agent's content
//!   is unavailable" from "our gateway had a bad day".

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

use crate::model::{CheckResult, CheckStatus};

/// What the prober observed for one agent's `tokenURI`, reduced to exactly
/// what this rung needs to judge it. Assembled by the sweeper from a
/// `probe::FetchOutcome`; this crate never learns how the bytes were
/// obtained.
#[derive(Debug, Clone)]
pub struct ResolvableInput {
    /// The raw, unresolved URI as declared on-chain.
    pub uri: String,
    /// The scheme bucket: `"empty"`, `"unsupported"`, `"data"`, `"http"`,
    /// `"https"`, or `"ipfs"`.
    pub scheme: String,
    pub request_url: Option<String>,
    pub final_url: Option<String>,
    pub http_status: Option<u16>,
    pub elapsed_ms: Option<u32>,
    /// Plain text describing OUR failure to get a usable response — timeout,
    /// DNS, TLS, robots.txt disallow/unreachable, IPFS gateway failure.
    /// Never a verdict; this rung is the only place that turns it into one.
    pub error: Option<String>,
    /// Decoded byte count for a `data:` URI. Only meaningful when
    /// `scheme == "data"`.
    pub inline_bytes: Option<usize>,
    /// Which IPFS gateway served this, when one did.
    pub via_gateway: Option<String>,
}

pub fn resolvable(input: &ResolvableInput, now: DateTime<Utc>) -> CheckResult {
    let mut evidence = Map::new();
    evidence.insert("uri".into(), json!(input.uri));
    evidence.insert("scheme".into(), json!(input.scheme));

    let status = match input.scheme.as_str() {
        "empty" => {
            evidence.insert("reason".into(), json!("no_uri"));
            CheckStatus::Fail
        }
        "unsupported" => {
            evidence.insert("reason".into(), json!("unsupported_scheme"));
            CheckStatus::Fail
        }
        "data" => {
            // The document is already in hand — no request was ever made, so
            // no HTTP field is populated, on purpose (see module doc).
            evidence.insert("inline".into(), json!(true));
            evidence.insert("bytes".into(), json!(input.inline_bytes));
            CheckStatus::Pass
        }
        _ => {
            // http, https, ipfs: a fetch was attempted. Record whichever of
            // the HTTP fields apply — including on failure, so a failing
            // rung still says what it saw.
            if let Some(v) = &input.request_url {
                evidence.insert("request_url".into(), json!(v));
            }
            if let Some(v) = &input.final_url {
                evidence.insert("final_url".into(), json!(v));
            }
            if let Some(v) = input.http_status {
                evidence.insert("http_status".into(), json!(v));
            }
            if let Some(v) = input.elapsed_ms {
                evidence.insert("elapsed_ms".into(), json!(v));
            }
            if let Some(v) = &input.via_gateway {
                evidence.insert("via_gateway".into(), json!(v));
            }

            if let Some(err) = &input.error {
                // OUR failure: we could not establish permission, reach the
                // host, complete TLS, or get an answer from the gateway.
                // That is never the agent's fault.
                evidence.insert("reason".into(), json!(err));
                CheckStatus::Error
            } else if let Some(code) = input.http_status {
                if (200..300).contains(&code) {
                    CheckStatus::Pass
                } else if code == 402 {
                    // A payment challenge is not the registration document.
                    evidence.insert("reason".into(), json!("payment_required"));
                    CheckStatus::Fail
                } else {
                    evidence.insert("reason".into(), json!("http_status"));
                    CheckStatus::Fail
                }
            } else {
                // Neither an error nor a status: the prober should always
                // give us one or the other for a scheme it attempted to
                // fetch. Treat the gap as OUR bug, never a silent pass.
                evidence.insert("reason".into(), json!("no_response"));
                CheckStatus::Error
            }
        }
    };

    CheckResult { rung: 2, name: "resolvable", status, evidence: Value::Object(evidence), checked_at: now }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CheckStatus;
    use chrono::{DateTime, Utc};

    fn t() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    /// A baseline HTTP 200 pass — most tests mutate one field off this.
    fn http_pass_input() -> ResolvableInput {
        ResolvableInput {
            uri: "https://example.com/agent.json".into(),
            scheme: "https".into(),
            request_url: Some("https://example.com/agent.json".into()),
            final_url: Some("https://example.com/agent.json".into()),
            http_status: Some(200),
            elapsed_ms: Some(120),
            error: None,
            inline_bytes: None,
            via_gateway: None,
        }
    }

    #[test]
    fn empty_scheme_fails_as_no_uri() {
        let i = ResolvableInput {
            uri: "".into(),
            scheme: "empty".into(),
            request_url: None,
            final_url: None,
            http_status: None,
            elapsed_ms: None,
            error: None,
            inline_bytes: None,
            via_gateway: None,
        };
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["reason"], "no_uri");
        assert_eq!(r.evidence["uri"], "");
        assert_eq!(r.evidence["scheme"], "empty");
    }

    #[test]
    fn unsupported_scheme_fails() {
        let mut i = http_pass_input();
        i.scheme = "unsupported".into();
        i.uri = "ftp://example.com/agent.json".into();
        i.request_url = None;
        i.final_url = None;
        i.http_status = None;
        i.elapsed_ms = None;
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["reason"], "unsupported_scheme");
    }

    #[test]
    fn data_uri_passes_and_carries_no_http_fields() {
        let i = ResolvableInput {
            uri: "data:application/json;base64,eyJhIjoxfQ==".into(),
            scheme: "data".into(),
            request_url: None,
            final_url: None,
            http_status: None,
            elapsed_ms: None,
            error: None,
            inline_bytes: Some(9),
            via_gateway: None,
        };
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["inline"], true);
        assert_eq!(r.evidence["bytes"], 9);
        // The document was already in hand — no fetch happened, so none of
        // these keys should even be present, not even as null.
        assert!(r.evidence.get("request_url").is_none());
        assert!(r.evidence.get("final_url").is_none());
        assert!(r.evidence.get("http_status").is_none());
        assert!(r.evidence.get("elapsed_ms").is_none());
    }

    #[test]
    fn http_2xx_passes() {
        let r = resolvable(&http_pass_input(), t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["http_status"], 200);
        assert_eq!(r.evidence["request_url"], "https://example.com/agent.json");
    }

    #[test]
    fn http_402_fails_as_payment_required_not_as_unparseable() {
        // A payment challenge is not the registration document — we cannot
        // parse a document we never received.
        let mut i = http_pass_input();
        i.http_status = Some(402);
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["reason"], "payment_required");
        assert_eq!(r.evidence["http_status"], 402);
    }

    #[test]
    fn any_other_http_status_fails_with_the_generic_reason() {
        let mut i = http_pass_input();
        i.http_status = Some(404);
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["reason"], "http_status");
        assert_eq!(r.evidence["http_status"], 404);

        let mut i2 = http_pass_input();
        i2.http_status = Some(500);
        let r2 = resolvable(&i2, t());
        assert_eq!(r2.status, CheckStatus::Fail);
        assert_eq!(r2.evidence["reason"], "http_status");
    }

    #[test]
    fn a_timeout_is_our_error_never_the_agents_fail() {
        let mut i = http_pass_input();
        i.http_status = None;
        i.error = Some("timeout".into());
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Error);
        assert_eq!(r.evidence["reason"], "timeout");
    }

    #[test]
    fn dns_and_tls_failures_are_also_our_error() {
        for reason in ["dns", "tls"] {
            let mut i = http_pass_input();
            i.http_status = None;
            i.error = Some(reason.into());
            let r = resolvable(&i, t());
            assert_eq!(r.status, CheckStatus::Error, "{reason} should be Error");
        }
    }

    #[test]
    fn robots_denied_is_error_and_never_fail() {
        // We could not establish permission to fetch — that is not a fact
        // about the agent's document.
        let mut i = http_pass_input();
        i.http_status = None;
        i.error = Some("robots_denied".into());
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Error);
        assert_ne!(r.status, CheckStatus::Fail);
    }

    #[test]
    fn a_gateway_failure_is_our_error() {
        let mut i = http_pass_input();
        i.scheme = "ipfs".into();
        i.http_status = None;
        i.error = Some("gateway".into());
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Error);
    }

    #[test]
    fn ipfs_pass_records_via_gateway() {
        let mut i = http_pass_input();
        i.scheme = "ipfs".into();
        i.uri = "ipfs://bafybeigdyrzt.../agent.json".into();
        i.via_gateway = Some("ipfs.io".into());
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["via_gateway"], "ipfs.io");
    }

    #[test]
    fn evidence_carries_uri_and_scheme_even_on_failure() {
        let mut i = http_pass_input();
        i.http_status = Some(500);
        let r = resolvable(&i, t());
        assert_eq!(r.evidence["uri"], i.uri);
        assert_eq!(r.evidence["scheme"], "https");
        assert_eq!(r.status, CheckStatus::Fail);
    }
}
