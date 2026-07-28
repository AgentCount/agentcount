//! Rung 4 — `conformant`: does the parsed document contain every field the
//! spec marks REQUIRED for the agent registration file?
//!
//! The field list is not this module's to decide. It is extracted, with
//! spec line citations and two ruled-on ambiguities, in
//! `spec/REQUIRED_FIELDS.md`. This file checks presence only, exactly the
//! fields that document lists, in the order it lists them — nothing added,
//! nothing inferred from outside knowledge of ERC-8004.
//!
//! **Presence, not type.** A key existing is all this rung asks. Whether
//! `services` actually holds an array, or `x402Support` actually holds a
//! boolean, is a question the field-list extraction explicitly deferred
//! (`REQUIRED_FIELDS.md` records the spec's stated types precisely so a
//! later rung can check them) — enforcing it here would fail agents against
//! a rule this rung never published.
//!
//! **`null` counts as missing.** `serde_json::Value::get` returns `Some` for
//! a JSON `null`, but a null `name` is not a name any more than an absent
//! one is, so presence is `get(..).is_some_and(|v| !v.is_null())`.
//!
//! **A non-object document fails, it does not panic.** Rung 3 deliberately
//! passes bare JSON arrays, strings, and numbers through (see its module
//! doc) and leaves the "is this even shaped like a document" question to
//! this rung. `serde_json::Value::get(&str)` is defined on every variant —
//! it simply returns `None` for anything that is not an object — so no
//! explicit `is_object()` guard is needed: all seven top-level fields read
//! as absent on a non-object document, which fails the rung via the same
//! `fields_missing` path as an object that genuinely omits them, with no
//! risk of a panic.

use chrono::{DateTime, Utc};
use serde_json::json;

use crate::model::{CheckResult, CheckStatus};

/// The parsed document rung 3 handed back. This crate never parses JSON
/// itself — see `rung3_parseable`'s module doc for why.
#[derive(Debug, Clone)]
pub struct ConformantInput {
    pub document: serde_json::Value,
}

/// The seven fields the registration file's governing MUST (spec line 54)
/// requires unconditionally. See `spec/REQUIRED_FIELDS.md` §"Unconditionally
/// REQUIRED" — this list must stay in lockstep with that file, not with the
/// spec directly.
pub const UNCONDITIONAL_FIELDS: [&str; 7] = [
    "type",
    "name",
    "description",
    "image",
    "services",
    "x402Support",
    "active",
];

/// The two sub-fields spec line 123 ("all fields in the registration are
/// mandatory") requires on every entry of `registrations`, when that array
/// is present at all. See `spec/REQUIRED_FIELDS.md` §"Conditionally
/// REQUIRED".
pub const REGISTRATION_ENTRY_FIELDS: [&str; 2] = ["agentId", "agentRegistry"];

/// A key is "present" only if it exists and is not JSON `null`.
fn is_present(value: &serde_json::Value, key: &str) -> bool {
    value.get(key).is_some_and(|v| !v.is_null())
}

pub fn conformant(input: &ConformantInput, spec_commit: &str, now: DateTime<Utc>) -> CheckResult {
    let doc = &input.document;

    let mut fields_found: Vec<&str> = Vec::new();
    let mut fields_missing: Vec<String> = Vec::new();
    for field in UNCONDITIONAL_FIELDS {
        if is_present(doc, field) {
            fields_found.push(field);
        } else {
            fields_missing.push(field.to_string());
        }
    }

    // `registrations` itself is a SHOULD (spec line 123), not a MUST — its
    // absence never fails this rung (REQUIRED_FIELDS.md Ruling 2). When
    // present, every entry's `agentId` and `agentRegistry` are mandatory.
    // A non-array `registrations` (e.g. a lone object) has no entries to
    // walk, so nothing is checked and nothing counts against the document —
    // the same "presence, not shape" restraint this rung applies everywhere
    // else; a later rung can decide whether that shape itself is wrong.
    let mut registrations_checked: u64 = 0;
    if let Some(entries) = doc
        .get("registrations")
        .filter(|v| !v.is_null())
        .and_then(|v| v.as_array())
    {
        for (i, entry) in entries.iter().enumerate() {
            registrations_checked += 1;
            for field in REGISTRATION_ENTRY_FIELDS {
                if !is_present(entry, field) {
                    fields_missing.push(format!("registrations[{i}].{field}"));
                }
            }
        }
    }

    let status = if fields_missing.is_empty() {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    };

    let evidence = json!({
        "fields_found": fields_found,
        "fields_missing": fields_missing,
        "spec_commit": spec_commit,
        "registrations_checked": registrations_checked,
    });

    CheckResult {
        rung: 4,
        name: "conformant",
        status,
        evidence,
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

    const SPEC_COMMIT: &str = "68fc6765761a10fb26f0692df21c8a6f9d12b1be";

    /// A document carrying all seven unconditional fields and nothing else —
    /// the minimal fully-conformant document, no `registrations` key.
    fn complete_document() -> serde_json::Value {
        json!({
            "type": "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",
            "name": "myAgentName",
            "description": "A natural language description of the Agent",
            "image": "https://example.com/agentimage.png",
            "services": [],
            "x402Support": false,
            "active": true,
        })
    }

    fn input(document: serde_json::Value) -> ConformantInput {
        ConformantInput { document }
    }

    #[test]
    fn all_seven_fields_present_passes() {
        let r = conformant(&input(complete_document()), SPEC_COMMIT, t());
        assert_eq!(r.rung, 4);
        assert_eq!(r.name, "conformant");
        assert_eq!(r.status, CheckStatus::Pass);
        let found: Vec<&str> = r.evidence["fields_found"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        for field in UNCONDITIONAL_FIELDS {
            assert!(found.contains(&field), "expected {field} in fields_found");
        }
        assert!(r.evidence["fields_missing"].as_array().unwrap().is_empty());
        assert_eq!(r.evidence["spec_commit"], SPEC_COMMIT);
        assert_eq!(r.evidence["registrations_checked"], 0);
    }

    #[test]
    fn each_unconditional_field_missing_individually_fails_naming_exactly_that_field() {
        for field in UNCONDITIONAL_FIELDS {
            let mut doc = complete_document();
            doc.as_object_mut().unwrap().remove(field);
            let r = conformant(&input(doc), SPEC_COMMIT, t());
            assert_eq!(
                r.status,
                CheckStatus::Fail,
                "removing {field} should fail the rung"
            );
            let missing: Vec<&str> = r.evidence["fields_missing"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(
                missing,
                vec![field],
                "removing {field} should name exactly that field"
            );
        }
    }

    #[test]
    fn a_null_value_counts_as_missing_not_present() {
        let mut doc = complete_document();
        doc["name"] = serde_json::Value::Null;
        let r = conformant(&input(doc), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Fail);
        let missing: Vec<&str> = r.evidence["fields_missing"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(missing, vec!["name"]);
        let found: Vec<&str> = r.evidence["fields_found"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(!found.contains(&"name"));
    }

    #[test]
    fn a_missing_registrations_key_passes_with_zero_checked() {
        let doc = complete_document(); // no "registrations" key at all
        let r = conformant(&input(doc), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["registrations_checked"], 0);
    }

    #[test]
    fn a_complete_registrations_entry_passes() {
        let mut doc = complete_document();
        doc["registrations"] = json!([
            { "agentId": 22, "agentRegistry": "eip155:1:0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1" }
        ]);
        let r = conformant(&input(doc), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["registrations_checked"], 1);
    }

    #[test]
    fn a_registrations_entry_missing_agent_registry_fails() {
        let mut doc = complete_document();
        doc["registrations"] = json!([
            { "agentId": 22 }
        ]);
        let r = conformant(&input(doc), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["registrations_checked"], 1);
        let missing: Vec<&str> = r.evidence["fields_missing"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(missing, vec!["registrations[0].agentRegistry"]);
    }

    #[test]
    fn all_seven_present_but_a_registration_entry_missing_agent_id_still_fails() {
        // Guards against a bug where the top-level pass short-circuits the
        // registrations walk.
        let mut doc = complete_document();
        doc["registrations"] = json!([
            { "agentRegistry": "eip155:1:0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1" }
        ]);
        let r = conformant(&input(doc), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Fail);
        let missing: Vec<&str> = r.evidence["fields_missing"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(missing, vec!["registrations[0].agentId"]);
    }

    #[test]
    fn a_second_registration_entry_missing_a_field_is_named_by_its_own_index() {
        let mut doc = complete_document();
        doc["registrations"] = json!([
            { "agentId": 1, "agentRegistry": "eip155:1:0xabc" },
            { "agentId": 2 },
        ]);
        let r = conformant(&input(doc), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["registrations_checked"], 2);
        let missing: Vec<&str> = r.evidence["fields_missing"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(missing, vec!["registrations[1].agentRegistry"]);
    }

    #[test]
    fn a_json_array_document_fails_rather_than_panics() {
        let r = conformant(&input(json!([1, 2, 3])), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Fail);
        let missing: Vec<&str> = r.evidence["fields_missing"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        for field in UNCONDITIONAL_FIELDS {
            assert!(missing.contains(&field));
        }
    }

    #[test]
    fn a_json_string_document_fails_rather_than_panics() {
        let r = conformant(&input(json!("just a string")), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["fields_found"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn a_json_number_document_fails_rather_than_panics() {
        let r = conformant(&input(json!(42)), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["fields_found"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn evidence_carries_the_spec_commit_on_both_pass_and_fail() {
        let pass = conformant(&input(complete_document()), SPEC_COMMIT, t());
        assert_eq!(pass.evidence["spec_commit"], SPEC_COMMIT);

        let mut doc = complete_document();
        doc.as_object_mut().unwrap().remove("active");
        let fail = conformant(&input(doc), SPEC_COMMIT, t());
        assert_eq!(fail.evidence["spec_commit"], SPEC_COMMIT);
    }
}
