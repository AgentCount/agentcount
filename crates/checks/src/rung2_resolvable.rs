//! Rung 2 — `resolvable`: can the agent's declared registration document be
//! retrieved at all?
//!
//! `Fail` is reserved for facts about what the agent published: no `tokenURI`,
//! a scheme we don't support, a URI that does not resolve or resolves only to
//! a private/loopback/link-local address (`ssrf_blocked: ...` — no third
//! party could have retrieved it either), or an HTTP status that means the
//! document itself is unavailable (402, 404, 500, ...). `Error` is reserved
//! for OUR limitations: a timeout, a TLS failure, robots.txt being
//! unreachable or disallowing us, or our IPFS gateway failing. Getting this
//! backwards publishes a false accusation about a real project, so every
//! branch below is judged against that line, not against "did we get a 200".
//!
//! Three decisions worth spelling out because they are easy to get wrong:
//!
//! * **HTTP 402 fails.** A payment challenge is not the registration
//!   document, and you cannot parse a document you never received. The
//!   status and (whatever) body are still archived by the prober; this rung
//!   only records that resolution failed and why.
//! * **`data:` URIs pass, unconditionally — UNLESS we understood the
//!   declared encoding but couldn't decode it.** 15,495 agents (25.8% of the
//!   registry) publish their registration document inline, and the spec
//!   explicitly permits it (`ERC8004SPEC.md` L52). An inline URI is the
//!   strongest possible answer to "can this be retrieved" — the document is
//!   already in hand, no network round trip needed. Evidence for this case
//!   carries `inline: true`, the decoded byte count, and — P0 FIX 7 — which
//!   of the five decode fallback paths produced the bytes
//!   (`data_uri_variant`, `data_uri_algorithm`), and deliberately carries no
//!   HTTP fields (`request_url`, `final_url`, `http_status`, `elapsed_ms`) —
//!   none of them were ever populated, and printing them as `null` would
//!   look like a fetch was attempted when it wasn't. The one exception: an
//!   `enc=` compression algorithm we don't implement (only `gzip` is) is
//!   OUR limitation, not the agent's — `error`, never `fail` — see the
//!   `"data"` match arm below.
//! * **`ipfs://` is tried against up to three gateways in sequence** (P0
//!   FIX 8, reversing an earlier ruling that used one disclosed gateway so a
//!   failure would be honestly attributable — the owner confirmed the
//!   reversal). The evidence records every gateway attempted
//!   (`gateway_attempts`) and which one won (`via_gateway`), so a reader can
//!   distinguish "this agent's content is unavailable everywhere we looked"
//!   from "one gateway had a bad day." All three failing is `error`, never
//!   `fail` — we cannot distinguish an unpinned CID from a network problem.

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
    /// Plain text describing why no usable response came back — timeout,
    /// TLS, robots.txt disallow/unreachable, IPFS gateway failure (all OURS),
    /// or an `ssrf_blocked: ...` netguard rejection (the agent's fact, since
    /// no third party could have fetched it either). Never a verdict; this
    /// rung is the only place that turns it into one.
    pub error: Option<String>,
    /// Decoded byte count for a `data:` URI. Only meaningful when
    /// `scheme == "data"`.
    pub inline_bytes: Option<usize>,
    /// Which IPFS gateway served this, when one did.
    pub via_gateway: Option<String>,
    /// P0 FIX 7: which of the five `data:` decode fallback paths produced
    /// `inline_bytes` — `"compressed"`, `"base64"`, `"plain"`,
    /// `"base64_claimed_but_raw_json"`, or `"plain_json_without_uri_scheme"`.
    /// Only meaningful when `scheme == "data"`.
    pub inline_decode_variant: Option<String>,
    /// The `enc=` algorithm name, set only when `inline_decode_variant ==
    /// Some("compressed")`.
    pub inline_decode_algorithm: Option<String>,
    /// P0 FIX 8: every IPFS gateway attempted, in order, with each one's own
    /// status — set only when `scheme == "ipfs"`. `None` for every other
    /// scheme, and for an `ipfs://` URI too malformed to try any gateway at
    /// all.
    pub gateway_attempts: Option<Value>,
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
            if let Some(v) = &input.inline_decode_variant {
                evidence.insert("data_uri_variant".into(), json!(v));
            }
            if let Some(v) = &input.inline_decode_algorithm {
                evidence.insert("data_uri_algorithm".into(), json!(v));
            }
            match &input.error {
                Some(err) => {
                    // P0 FIX 7: we understood the declared encoding (an
                    // `enc=` compression algorithm) but could not decode
                    // it — OUR limitation, never the agent's document being
                    // at fault.
                    evidence.insert("reason".into(), json!(err));
                    CheckStatus::Error
                }
                None => {
                    evidence.insert("bytes".into(), json!(input.inline_bytes));
                    CheckStatus::Pass
                }
            }
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
            if let Some(v) = &input.gateway_attempts {
                // P0 FIX 8: the whole chain, not just the winner.
                evidence.insert("gateway_attempts".into(), v.clone());
            }

            if let Some(err) = &input.error {
                if let Some(reason) = err.strip_prefix("ssrf_blocked: ") {
                    // The netguard is why WE never attempted the request —
                    // but the reason it was unattemptable (doesn't resolve,
                    // or resolves only to a private/loopback/link-local
                    // address) is a fact about the URI the agent published,
                    // not a limitation of ours. No third party could have
                    // retrieved this document either; that's the same
                    // category as an empty URI, which already fails. Keep
                    // `robots_unavailable`/`robots_disallowed` out of this
                    // branch: there we genuinely could not establish
                    // permission, and that constraint is ours.
                    evidence.insert("reason".into(), json!(format!("ssrf_blocked: {reason}")));
                    CheckStatus::Fail
                } else {
                    // OUR failure: we could not establish permission, reach
                    // the host, complete TLS, or get an answer from the
                    // gateway. That is never the agent's fault.
                    evidence.insert("reason".into(), json!(err));
                    CheckStatus::Error
                }
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

    CheckResult {
        rung: 2,
        name: "resolvable",
        status,
        evidence: Value::Object(evidence),
        checked_at: now,
    }
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
            inline_decode_variant: None,
            inline_decode_algorithm: None,
            gateway_attempts: None,
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
            inline_decode_variant: None,
            inline_decode_algorithm: None,
            gateway_attempts: None,
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
            inline_decode_variant: Some("base64".into()),
            inline_decode_algorithm: None,
            gateway_attempts: None,
        };
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["inline"], true);
        assert_eq!(r.evidence["bytes"], 9);
        assert_eq!(r.evidence["data_uri_variant"], "base64");
        // The document was already in hand — no fetch happened, so none of
        // these keys should even be present, not even as null.
        assert!(r.evidence.get("request_url").is_none());
        assert!(r.evidence.get("final_url").is_none());
        assert!(r.evidence.get("http_status").is_none());
        assert!(r.evidence.get("elapsed_ms").is_none());
    }

    // --- P0 FIX 7: an unsupported `data:` compression algorithm ----------

    #[test]
    fn an_unsupported_data_uri_compression_algorithm_is_error_not_fail() {
        let i = ResolvableInput {
            uri: "data:application/json;enc=zstd;base64,eyJhIjoxfQ==".into(),
            scheme: "data".into(),
            request_url: None,
            final_url: None,
            http_status: None,
            elapsed_ms: None,
            error: Some("unsupported_compression: zstd".into()),
            inline_bytes: None,
            via_gateway: None,
            inline_decode_variant: None,
            inline_decode_algorithm: None,
            gateway_attempts: None,
        };
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Error);
        assert_eq!(r.evidence["reason"], "unsupported_compression: zstd");
        assert!(
            r.evidence.get("bytes").is_none(),
            "there is nothing decoded to report a byte count for"
        );
    }

    #[test]
    fn a_gzip_compressed_data_uri_passes_and_records_the_algorithm() {
        let mut i = ResolvableInput {
            uri: "data:application/json;enc=gzip;level=6;base64,H4sI...".into(),
            scheme: "data".into(),
            request_url: None,
            final_url: None,
            http_status: None,
            elapsed_ms: None,
            error: None,
            inline_bytes: Some(42),
            via_gateway: None,
            inline_decode_variant: Some("compressed".into()),
            inline_decode_algorithm: Some("gzip".into()),
            gateway_attempts: None,
        };
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["data_uri_variant"], "compressed");
        assert_eq!(r.evidence["data_uri_algorithm"], "gzip");

        // Sanity: every other named variant round-trips into evidence too.
        for variant in [
            "plain",
            "base64_claimed_but_raw_json",
            "plain_json_without_uri_scheme",
        ] {
            i.inline_decode_variant = Some(variant.into());
            i.inline_decode_algorithm = None;
            let r = resolvable(&i, t());
            assert_eq!(r.evidence["data_uri_variant"], variant);
            assert!(r.evidence.get("data_uri_algorithm").is_none());
        }
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
    fn robots_unavailable_is_still_error() {
        // A robots.txt we could not establish permission for (5xx, timeout,
        // connection failure, or an unfollowable redirect) is OUR
        // limitation, never the agent's.
        let mut i = http_pass_input();
        i.http_status = None;
        i.error = Some("robots_unavailable: robots.txt returned HTTP 503".into());
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Error);
        assert_eq!(
            r.evidence["reason"],
            "robots_unavailable: robots.txt returned HTTP 503"
        );
    }

    #[test]
    fn robots_disallowed_is_still_error() {
        let mut i = http_pass_input();
        i.http_status = None;
        i.error = Some("robots_disallowed".into());
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Error);
        assert_eq!(r.evidence["reason"], "robots_disallowed");
    }

    #[test]
    fn dns_resolution_failure_via_the_netguard_fails_not_errors() {
        // The project owner's ruling: an agentURI that does not resolve is a
        // fact about what the agent published — the same category as an
        // empty URI (which already fails) — not a limitation of ours.
        let mut i = http_pass_input();
        i.http_status = None;
        i.error = Some("ssrf_blocked: dns resolution failed: no record found".into());
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(
            r.evidence["reason"],
            "ssrf_blocked: dns resolution failed: no record found"
        );
    }

    #[test]
    fn non_public_address_via_the_netguard_fails_not_errors() {
        let mut i = http_pass_input();
        i.http_status = None;
        i.error = Some("ssrf_blocked: resolves to a non-public address".into());
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(
            r.evidence["reason"],
            "ssrf_blocked: resolves to a non-public address"
        );
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
        i.via_gateway = Some("https://ipfs.io/ipfs/".into());
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["via_gateway"], "https://ipfs.io/ipfs/");
    }

    // --- P0 FIX 8: IPFS gateway fallback chain ----------------------------

    #[test]
    fn ipfs_pass_records_the_whole_gateway_chain_not_just_the_winner() {
        let mut i = http_pass_input();
        i.scheme = "ipfs".into();
        i.uri = "ipfs://bafybeigdyrzt.../agent.json".into();
        i.via_gateway = Some("https://cloudflare-ipfs.com/ipfs/".into());
        i.gateway_attempts = Some(json!([
            {"gateway": "https://ipfs.io/ipfs/", "http_status": 404, "error": null},
            {"gateway": "https://cloudflare-ipfs.com/ipfs/", "http_status": 200, "error": null},
        ]));
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["gateway_attempts"].as_array().unwrap().len(), 2);
        assert_eq!(r.evidence["gateway_attempts"][0]["http_status"], 404);
        assert_eq!(r.evidence["gateway_attempts"][1]["http_status"], 200);
    }

    #[test]
    fn all_three_ipfs_gateways_failing_is_error_not_fail_with_the_full_chain_recorded() {
        // We cannot distinguish an unpinned CID from a network problem on
        // our end — see P0 FIX 8. Must be `Error`, never `Fail`, even though
        // every individual attempt got a definite (non-2xx) HTTP status.
        let mut i = http_pass_input();
        i.scheme = "ipfs".into();
        i.uri = "ipfs://bafybeigdyrzt.../agent.json".into();
        i.http_status = None;
        i.via_gateway = None;
        i.error = Some("ipfs_all_gateways_failed".into());
        i.gateway_attempts = Some(json!([
            {"gateway": "https://ipfs.io/ipfs/", "http_status": 404, "error": null},
            {"gateway": "https://cloudflare-ipfs.com/ipfs/", "http_status": 404, "error": null},
            {"gateway": "https://gateway.pinata.cloud/ipfs/", "http_status": 522, "error": null},
        ]));
        let r = resolvable(&i, t());
        assert_eq!(r.status, CheckStatus::Error);
        assert_eq!(r.evidence["reason"], "ipfs_all_gateways_failed");
        assert_eq!(r.evidence["gateway_attempts"].as_array().unwrap().len(), 3);
        assert!(r.evidence.get("via_gateway").is_none());
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
