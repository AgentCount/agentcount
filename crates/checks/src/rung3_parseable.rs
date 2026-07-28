//! Rung 3 — `parseable`: does the retrieved document parse as JSON at all?
//!
//! This rung asks nothing more than "is this valid JSON" — not "is it an
//! object", not "does it have the required fields". Deciding shape is rung
//! 4's job; conflating the two here would mean a bare JSON array or string
//! fails a question this rung never asked.
//!
//! `Fail` vs `Error`, the same line rung 2 draws: `Fail` covers malformed
//! JSON, because that is a fact about what the agent published. `Error`
//! covers OUR limitations — specifically, a body we truncated ourselves at
//! [`probe::MAX_BODY_BYTES`]. Judging a truncated body "invalid JSON" would
//! report our cap as their malformity, so a truncated body never reaches
//! `serde_json::from_slice` at all.
//!
//! **Content-type is recorded, never enforced.** The spec does not require
//! `application/json`, and failing an agent over a header the spec never
//! mandated would be inventing a rule it never agreed to. A document served
//! as `text/plain` that parses cleanly passes exactly like one served as
//! `application/json`.
//!
//! **A leading UTF-8 BOM is stripped before parsing.** RFC 8259 §8.1 permits
//! (without requiring) a JSON text to begin with a byte-order mark, and says
//! implementations MAY ignore it rather than treat it as an error. We take
//! that option: a BOM is an artifact of how a document was encoded, not a
//! structural defect in the JSON itself, and it is exactly the same kind of
//! "don't invent a rule the spec doesn't have" judgment as the content-type
//! decision above. `body_sha256` still covers the untouched bytes, so the
//! archived evidence remains reproducible.
//!
//! The parsed [`serde_json::Value`] travels back alongside the [`CheckResult`]
//! so rungs 4 and 5 parse the body exactly once and can never disagree with
//! this rung about whether it parsed.

use chrono::{DateTime, Utc};
use serde_json::json;

use crate::model::{CheckResult, CheckStatus};

/// What rung 2 (`resolvable`) handed forward: the body it fetched, or
/// decoded from a `data:` URI, reduced to what this rung needs to judge it.
#[derive(Debug, Clone)]
pub struct ParseableInput {
    pub body: Option<Vec<u8>>,
    /// Recorded in evidence, never used to gate the verdict — see module doc.
    pub content_type: Option<String>,
    pub body_sha256: Option<String>,
    /// Set when the prober cut the body off at its size cap. OUR limit, not
    /// the agent's malformity.
    pub truncated: bool,
}

/// A leading UTF-8 byte-order mark, stripped before parsing per RFC 8259 §8.1.
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

pub fn parseable(
    input: &ParseableInput,
    now: DateTime<Utc>,
) -> (CheckResult, Option<serde_json::Value>) {
    let mut evidence = json!({
        "content_type": input.content_type,
        "body_sha256": input.body_sha256,
        "body_bytes": input.body.as_ref().map(Vec::len),
    });

    if input.truncated {
        // We capped it ourselves; judging the (possibly cut-mid-token) JSON
        // invalid would be our bug reported as their failure.
        evidence["reason"] = json!("body_truncated");
        let result = CheckResult {
            rung: 3,
            name: "parseable",
            status: CheckStatus::Error,
            evidence,
            checked_at: now,
        };
        return (result, None);
    }

    let Some(body) = &input.body else {
        // Defence in depth: rung 2 should already have stopped the ladder
        // before we get here with no body at all.
        evidence["reason"] = json!("no_body");
        let result = CheckResult {
            rung: 3,
            name: "parseable",
            status: CheckStatus::Error,
            evidence,
            checked_at: now,
        };
        return (result, None);
    };

    let to_parse: &[u8] = body.strip_prefix(&UTF8_BOM).unwrap_or(body.as_slice());

    match serde_json::from_slice::<serde_json::Value>(to_parse) {
        Ok(value) => {
            let result = CheckResult {
                rung: 3,
                name: "parseable",
                status: CheckStatus::Pass,
                evidence,
                checked_at: now,
            };
            (result, Some(value))
        }
        Err(e) => {
            evidence["parse_error"] = json!(e.to_string());
            let result = CheckResult {
                rung: 3,
                name: "parseable",
                status: CheckStatus::Fail,
                evidence,
                checked_at: now,
            };
            (result, None)
        }
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

    fn input_with(body: &[u8]) -> ParseableInput {
        ParseableInput {
            body: Some(body.to_vec()),
            content_type: Some("application/json".into()),
            body_sha256: Some("deadbeef".into()),
            truncated: false,
        }
    }

    #[test]
    fn valid_json_passes_and_the_parsed_document_travels_with_the_result() {
        let (r, doc) = parseable(&input_with(br#"{"name":"agent"}"#), t());
        assert_eq!(r.status, CheckStatus::Pass);
        let doc = doc.expect("a passing parse must hand back the document");
        assert_eq!(doc["name"], "agent");
    }

    #[test]
    fn invalid_json_fails_and_captures_serdes_error_with_line_and_column() {
        let (r, doc) = parseable(&input_with(b"{not json"), t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert!(doc.is_none());
        let msg = r.evidence["parse_error"]
            .as_str()
            .expect("parse_error must be a string");
        assert!(!msg.is_empty());
    }

    #[test]
    fn a_truncated_body_is_our_error_never_the_agents_fail() {
        let mut i = input_with(br#"{"name":"agen"#); // cut mid-token
        i.truncated = true;
        let (r, doc) = parseable(&i, t());
        assert_eq!(r.status, CheckStatus::Error);
        assert_eq!(r.evidence["reason"], "body_truncated");
        assert!(doc.is_none());
        // Must never masquerade as an agent-caused parse failure.
        assert!(r.evidence.get("parse_error").is_none());
    }

    #[test]
    fn a_missing_body_is_our_error_defence_in_depth() {
        let i = ParseableInput {
            body: None,
            content_type: None,
            body_sha256: None,
            truncated: false,
        };
        let (r, doc) = parseable(&i, t());
        assert_eq!(r.status, CheckStatus::Error);
        assert_eq!(r.evidence["reason"], "no_body");
        assert!(doc.is_none());
        assert!(r.evidence["body_bytes"].is_null());
    }

    #[test]
    fn valid_json_served_as_text_plain_still_passes_content_type_is_not_enforced() {
        let mut i = input_with(br#"{"name":"agent"}"#);
        i.content_type = Some("text/plain".into());
        let (r, _doc) = parseable(&i, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["content_type"], "text/plain");
    }

    #[test]
    fn a_leading_utf8_bom_is_stripped_and_the_document_still_parses() {
        let mut body = UTF8_BOM.to_vec();
        body.extend_from_slice(br#"{"name":"agent"}"#);
        let (r, doc) = parseable(&input_with(&body), t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(doc.unwrap()["name"], "agent");
    }

    #[test]
    fn a_bare_json_array_passes_rung_3_leaves_the_object_shape_question_to_rung_4() {
        let (r, doc) = parseable(&input_with(b"[1,2,3]"), t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert!(doc.unwrap().is_array());
    }

    #[test]
    fn a_bare_json_string_also_passes() {
        let (r, doc) = parseable(&input_with(br#""just a string""#), t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert!(doc.unwrap().is_string());
    }

    #[test]
    fn evidence_always_carries_content_type_sha256_and_body_bytes() {
        let (r, _doc) = parseable(&input_with(br#"{"a":1}"#), t());
        assert_eq!(r.evidence["content_type"], "application/json");
        assert_eq!(r.evidence["body_sha256"], "deadbeef");
        assert_eq!(r.evidence["body_bytes"], 7);

        // And even on the failure path.
        let (r_fail, _) = parseable(&input_with(b"{bad"), t());
        assert_eq!(r_fail.evidence["content_type"], "application/json");
        assert_eq!(r_fail.evidence["body_sha256"], "deadbeef");
        assert!(r_fail.evidence["body_bytes"].is_u64());
    }
}
