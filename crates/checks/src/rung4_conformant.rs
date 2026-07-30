//! Rung 4 — `conformant`: does the parsed document violate anything the spec
//! actually marks MUST, and what does it fall short of at SHOULD/MAY?
//!
//! **P0 FIX 3 — RFC 2119 severity, not one bucket.** The spec invokes RFC
//! 2119/8174 explicitly (line 36): MUST, SHOULD, and MAY are three different
//! promises, and collapsing all three into a single `fail` was exactly the
//! compression this project exists to refuse — the same mistake committed by
//! the very product that argues against it. This rung now classifies every
//! field it looks at into one of three severities (see
//! `spec/REQUIRED_FIELDS.md` for the full citation trail) and reports all
//! three, always:
//!
//! - **`pass`** — zero MUST violations.
//! - **`fail`** — one or more MUST violations.
//! - Evidence always carries `must_violations[]`, `should_gaps[]`, and
//!   `may_gaps[]` — never just one of them, so a reader can always see the
//!   full severity breakdown regardless of which way the binary landed.
//!
//! **The uncomfortable finding, stated plainly, not softened.** Under an
//! honest reading of the pinned spec, the registration file has exactly
//! **one MUST**, and it is *conditional*: `registrations[].agentId` and
//! `registrations[].agentRegistry`, checked only when a `registrations`
//! array is present at all (spec line 123, "all fields in the registration
//! are mandatory" — but line 123 itself downgrades the array's own presence
//! to SHOULD, see `REQUIRED_FIELDS.md` Ruling 2). Everything else this rung
//! once treated as REQUIRED — `type`, `name`, `description`, `image`,
//! `services` — is SHOULD. `x402Support`, `active`, and `supportedTrust`
//! are MAY. This means the overwhelming majority of documents that parse as
//! JSON at all will now pass rung 4: ERC-8004 imposes almost no *hard*
//! requirement on this document. That is not a bug in this rung and it is
//! not softened here — it is the finding. The interesting number moves to
//! the SHOULD-completeness distribution (how many of the SHOULD fields a
//! document actually carries), which this evidence makes queryable without
//! a re-sweep.
//!
//! **`updatedAt` — a work-order field the pinned spec does not define, and
//! is deliberately NOT checked.** The P0 FIX 3 work order's MAY bucket
//! lists `x402Support`, `active`, `supportedTrust`, `updatedAt`. The first
//! three are in the pinned spec (`x402Support`/`active`: example JSON only,
//! lines 99-100; `supportedTrust`: line 124, explicitly OPTIONAL).
//! `updatedAt` is not: it does not appear anywhere in
//! `spec/ERC8004SPEC.md` at the pinned commit — not in the line-54 schema
//! block, not in prose, not with any normative keyword. Cross-checked
//! against both verification sources ground rule 1 names
//! (`eips.ethereum.org/EIPS/eip-8004` agrees the pinned text has no such
//! field) and `best-practices.8004scan.io`, which *does* define `updatedAt`
//! as an optional freshness-tracking timestamp — the work order's source
//! for this entry is evidently 8004scan's extended profile, not the pinned
//! spec. Ground rule 1 is explicit: "If they disagree with this document,
//! the spec wins and the disagreement is reported back, not silently
//! resolved." This rung's own stated discipline (see the original module
//! doc, preserved below) is "nothing added, nothing inferred from outside
//! knowledge of ERC-8004" — so `updatedAt` is not checked, in either
//! direction, and its absence from `MAY_FIELDS` is this citation, not an
//! oversight. See `spec/REQUIRED_FIELDS.md` for the full record.
//!
//! **Presence, not type.** Unchanged from before FIX 3: a key existing is
//! all any of these three checks ask. Whether `services` actually holds an
//! array, or `image` actually holds a URI, is a question this rung still
//! does not ask.
//!
//! **`services` accepts the legacy `endpoints` alias (P0 FIX 1).** Carried
//! forward unchanged in spirit, reclassified SHOULD by FIX 3: the schema
//! block (spec line 62) names the field `services`, prose still says
//! "endpoints" at lines 115, 117, 121, 402. The rule remains
//! `services.or(endpoints)`; which name (if either) was used is recorded in
//! `services_field_source`, `legacy_endpoints_field`, `both_fields_present`.
//!
//! **`services`: empty vs. absent, recorded distinctly (P0 FIX 3).** The
//! spec never marks `services` MUST, and 8004scan's profile phrases it
//! conditionally — required only if the agent is meant to be interacted
//! with. So presence is SHOULD, not MUST. But a document that *has* the key
//! with zero entries is a materially different fact than a document that
//! omits the key altogether: the first describes an agent nobody can reach
//! by any advertised means, the second simply never engaged with this part
//! of the schema. Conflating them would erase exactly the kind of
//! distinction this project exists to preserve, so `services_status`
//! (`"absent"` / `"empty"` / `"present"`) is always recorded, and the two
//! failure modes get distinct `should_gaps` labels: `"services"` for
//! absent, `"services_empty"` for present-but-empty.
//!
//! **`x402Support` and `active` are MAY (P0 FIX 2, formalized by FIX 3).**
//! Both keys appear only inside the spec's illustrative example JSON block
//! (lines 99 and 100) and are never mentioned in prose with a normative
//! keyword. FIX 2 removed them from the required set; FIX 3 gives that
//! removal a formal classification — MAY, alongside `supportedTrust`
//! (explicitly OPTIONAL, line 124).
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
//! explicit `is_object()` guard is needed. A non-object document therefore
//! reads every field as absent: no `registrations` array exists, so it
//! carries zero MUST violations (and thus *passes* rung 4 under FIX 3 — an
//! uncomfortable but honest consequence of the only MUST being conditional
//! on a key that, by definition, cannot be present on a non-object value;
//! it accumulates every SHOULD gap instead, which is where FIX 3 intends
//! that kind of document's deficiency to show up).

use chrono::{DateTime, Utc};
use serde_json::json;

use crate::model::{CheckResult, CheckStatus};

/// The parsed document rung 3 handed back. This crate never parses JSON
/// itself — see `rung3_parseable`'s module doc for why.
#[derive(Debug, Clone)]
pub struct ConformantInput {
    pub document: serde_json::Value,
}

/// The two sub-fields spec line 123 ("all fields in the registration are
/// mandatory") requires on every entry of `registrations`, when that array
/// is present at all. **This is the entire MUST set rung 4 checks** — see
/// `spec/REQUIRED_FIELDS.md` §"MUST". There is no unconditional MUST field
/// left after P0 FIX 3.
pub const REGISTRATION_ENTRY_FIELDS: [&str; 2] = ["agentId", "agentRegistry"];

/// The four top-level fields spec line 115 marks SHOULD ("ensure
/// compatibility with ERC-721 apps"). Checked for presence on every
/// document; a missing one is a `should_gaps` entry, never a MUST
/// violation. See `spec/REQUIRED_FIELDS.md` §"SHOULD" and Ruling 4 (the
/// FIX-3 reversal of Ruling 1, which had held these REQUIRED).
pub const SHOULD_TOP_LEVEL_FIELDS: [&str; 4] = ["type", "name", "description", "image"];

/// SHOULD-severity checks that need special handling beyond simple
/// presence — `services` (legacy-alias + empty-vs-absent), `registrations`
/// (at least one entry), and `services[].version` (per-entry, aggregated to
/// one gap label). Exposed as a named list purely for `/api/methodology` to
/// describe what these three cover; the checks themselves are not a simple
/// presence loop, see [`conformant`].
pub const SHOULD_SPECIAL_FIELDS: [&str; 3] = ["services", "registrations", "services[].version"];

/// MAY fields the pinned spec actually supports a classification for.
/// `x402Support`/`active`: example JSON only, no normative prose (lines 99,
/// 100). `supportedTrust`: explicitly OPTIONAL (line 124). **`updatedAt`,
/// which the P0 FIX 3 work order's MAY bucket also names, is deliberately
/// excluded** — it does not appear anywhere in the pinned spec; see the
/// module doc for the full citation trail. Its absence here is that
/// citation, not an oversight.
pub const MAY_FIELDS: [&str; 3] = ["x402Support", "active", "supportedTrust"];

/// A key is "present" only if it exists and is not JSON `null`.
fn is_present(value: &serde_json::Value, key: &str) -> bool {
    value.get(key).is_some_and(|v| !v.is_null())
}

pub fn conformant(input: &ConformantInput, spec_commit: &str, now: DateTime<Utc>) -> CheckResult {
    let doc = &input.document;

    // ---- SHOULD: the four top-level presence fields (spec line 115) -----
    let mut should_gaps: Vec<String> = Vec::new();
    for field in SHOULD_TOP_LEVEL_FIELDS {
        if !is_present(doc, field) {
            should_gaps.push(field.to_string());
        }
    }

    // ---- SHOULD (special): services/endpoints, empty vs. absent ---------
    // P0 FIX 1's alias carried forward: `services.or(endpoints)`. Never a
    // MUST — the spec doesn't mark it one, and 8004scan phrases it
    // conditionally (required only if the agent is meant to be interacted
    // with). P0 FIX 3 additionally distinguishes "the key is missing" from
    // "the key is there but empty" — the latter describes an unreachable
    // agent and deserves its own label, not silent conflation with absence.
    let services_value = doc.get("services").filter(|v| !v.is_null());
    let endpoints_value = doc.get("endpoints").filter(|v| !v.is_null());
    let has_services = services_value.is_some();
    let has_endpoints = endpoints_value.is_some();
    let both_fields_present = has_services && has_endpoints;
    let legacy_endpoints_field = !has_services && has_endpoints;
    let services_field_source = if has_services {
        "services"
    } else if has_endpoints {
        "endpoints"
    } else {
        "neither"
    };
    // Whichever name actually supplied the value, per FIX 1 precedence
    // (`services` wins when both present).
    let active_services_value = services_value.or(endpoints_value);
    let services_entries: Option<&Vec<serde_json::Value>> =
        active_services_value.and_then(|v| v.as_array());

    let services_status = match (active_services_value, services_entries) {
        (None, _) => "absent",
        (Some(_), Some(entries)) if entries.is_empty() => "empty",
        (Some(_), Some(_)) => "present",
        // Present but not an array at all (e.g. a lone object) — not a
        // shape this rung enforces (see module doc, "presence, not type"),
        // but it plainly is not usable as a non-empty service list either,
        // so it reads the same as "empty" for the purpose of this signal.
        (Some(_), None) => "empty",
    };
    match services_status {
        "absent" => should_gaps.push("services".to_string()),
        "empty" => should_gaps.push("services_empty".to_string()),
        _ => {}
    }

    // ---- SHOULD (special): services[].version (spec line 115) -----------
    // "The version field in endpoints is a SHOULD, not a MUST." Checked
    // against whichever entries actually exist (services_entries, already
    // resolved through the same alias). Aggregated to ONE gap label if any
    // entry lacks it, rather than one per index — this is a completeness
    // signal about the document, not a per-entry MUST violation list.
    if let Some(entries) = services_entries
        && entries.iter().any(|entry| !is_present(entry, "version"))
    {
        should_gaps.push("services[].version".to_string());
    }

    // ---- SHOULD (special): registrations, at least one (spec line 123) --
    // Ruling 2 (spec/REQUIRED_FIELDS.md): the array's own presence is
    // SHOULD, not MUST. "At least one" means both an absent key and a
    // present-but-empty array fail this SHOULD — one label covers both,
    // since the work order only asked for the absent/empty distinction on
    // `services`, not `registrations`.
    let registrations_entries: Option<&Vec<serde_json::Value>> = doc
        .get("registrations")
        .filter(|v| !v.is_null())
        .and_then(|v| v.as_array());
    let registrations_checked = registrations_entries.map(Vec::len).unwrap_or(0) as u64;
    if registrations_checked == 0 {
        should_gaps.push("registrations".to_string());
    }

    // ---- MUST: registrations[].agentId / agentRegistry, conditional -----
    // The ONLY MUST in this document (spec line 123, "all fields in the
    // registration are mandatory") — and only when `registrations` is
    // present as a non-empty-checkable array at all. An absent or empty
    // `registrations` contributes zero MUST violations; its deficiency is
    // captured above as a SHOULD gap instead.
    let mut must_violations: Vec<String> = Vec::new();
    if let Some(entries) = registrations_entries {
        for (i, entry) in entries.iter().enumerate() {
            for field in REGISTRATION_ENTRY_FIELDS {
                if !is_present(entry, field) {
                    must_violations.push(format!("registrations[{i}].{field}"));
                }
            }
        }
    }

    // ---- MAY: informational only, never affects pass/fail ---------------
    let mut may_gaps: Vec<String> = Vec::new();
    for field in MAY_FIELDS {
        if !is_present(doc, field) {
            may_gaps.push(field.to_string());
        }
    }

    let status = if must_violations.is_empty() {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    };

    let evidence = json!({
        "must_violations": must_violations,
        "should_gaps": should_gaps,
        "may_gaps": may_gaps,
        "spec_commit": spec_commit,
        "registrations_checked": registrations_checked,
        // P0 FIX 1 — which name (if either) supplied the services/endpoints
        // value, so the population migration rate is queryable later.
        "services_field_source": services_field_source,
        "legacy_endpoints_field": legacy_endpoints_field,
        "both_fields_present": both_fields_present,
        // P0 FIX 3 — absent vs. empty, recorded distinctly (see module doc).
        "services_status": services_status,
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

    /// A document carrying every SHOULD/MAY field, plus a complete
    /// `registrations` entry — the fully-conformant shape.
    fn complete_document() -> serde_json::Value {
        json!({
            "type": "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",
            "name": "myAgentName",
            "description": "A natural language description of the Agent",
            "image": "https://example.com/agentimage.png",
            "services": [
                { "name": "web", "endpoint": "https://web.agentxyz.com/", "version": "1.0" }
            ],
            "x402Support": false,
            "active": true,
            "supportedTrust": ["reputation"],
            "registrations": [
                { "agentId": 22, "agentRegistry": "eip155:1:0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1" }
            ],
        })
    }

    fn input(document: serde_json::Value) -> ConformantInput {
        ConformantInput { document }
    }

    fn strs(v: &serde_json::Value, key: &str) -> Vec<String> {
        v[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    }

    // ------------------------------------------------------------------
    // Baseline: the fully-complete document passes with empty gap arrays.
    // ------------------------------------------------------------------

    #[test]
    fn a_fully_complete_document_passes_with_no_gaps_anywhere() {
        let r = conformant(&input(complete_document()), SPEC_COMMIT, t());
        assert_eq!(r.rung, 4);
        assert_eq!(r.name, "conformant");
        assert_eq!(r.status, CheckStatus::Pass);
        assert!(strs(&r.evidence, "must_violations").is_empty());
        assert!(strs(&r.evidence, "should_gaps").is_empty());
        assert!(strs(&r.evidence, "may_gaps").is_empty());
        assert_eq!(r.evidence["spec_commit"], SPEC_COMMIT);
        assert_eq!(r.evidence["registrations_checked"], 1);
        assert_eq!(r.evidence["services_status"], "present");
    }

    #[test]
    fn evidence_always_carries_all_three_arrays_on_pass_and_fail() {
        for doc in [complete_document(), json!({})] {
            let r = conformant(&input(doc), SPEC_COMMIT, t());
            assert!(r.evidence.get("must_violations").is_some());
            assert!(r.evidence.get("should_gaps").is_some());
            assert!(r.evidence.get("may_gaps").is_some());
        }
    }

    // ------------------------------------------------------------------
    // MUST bucket — the only thing that can flip pass/fail. One fixture
    // per rule requested by the work order.
    // ------------------------------------------------------------------

    #[test]
    fn registrations_absent_is_no_must_violation_and_exactly_one_should_gap() {
        let doc = complete_document();
        let mut doc = doc;
        doc.as_object_mut().unwrap().remove("registrations");

        let r = conformant(&input(doc), SPEC_COMMIT, t());

        assert_eq!(
            r.status,
            CheckStatus::Pass,
            "no registrations key must never be a MUST violation"
        );
        assert!(strs(&r.evidence, "must_violations").is_empty());
        assert_eq!(strs(&r.evidence, "should_gaps"), vec!["registrations"]);
        assert_eq!(r.evidence["registrations_checked"], 0);
    }

    #[test]
    fn registrations_present_with_an_entry_missing_agent_registry_is_a_must_violation_and_fails() {
        let mut doc = complete_document();
        doc["registrations"] = json!([{ "agentId": 22 }]);

        let r = conformant(&input(doc), SPEC_COMMIT, t());

        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(
            strs(&r.evidence, "must_violations"),
            vec!["registrations[0].agentRegistry"]
        );
        // The array is non-empty, so it does NOT also count as the
        // "registrations" SHOULD gap — presence is satisfied, only the
        // MUST-severity content is missing.
        assert!(!strs(&r.evidence, "should_gaps").contains(&"registrations".to_string()));
    }

    #[test]
    fn registrations_present_with_an_entry_missing_agent_id_is_a_must_violation_and_fails() {
        let mut doc = complete_document();
        doc["registrations"] = json!([
            { "agentRegistry": "eip155:1:0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1" }
        ]);

        let r = conformant(&input(doc), SPEC_COMMIT, t());

        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(
            strs(&r.evidence, "must_violations"),
            vec!["registrations[0].agentId"]
        );
    }

    #[test]
    fn a_registrations_entry_missing_both_must_fields_reports_both() {
        let mut doc = complete_document();
        doc["registrations"] = json!([{}]);

        let r = conformant(&input(doc), SPEC_COMMIT, t());

        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(
            strs(&r.evidence, "must_violations"),
            vec!["registrations[0].agentId", "registrations[0].agentRegistry"]
        );
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
        assert_eq!(
            strs(&r.evidence, "must_violations"),
            vec!["registrations[1].agentRegistry"]
        );
    }

    #[test]
    fn a_null_registration_field_counts_as_a_must_violation_not_present() {
        let mut doc = complete_document();
        doc["registrations"] = json!([{ "agentId": 22, "agentRegistry": null }]);

        let r = conformant(&input(doc), SPEC_COMMIT, t());

        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(
            strs(&r.evidence, "must_violations"),
            vec!["registrations[0].agentRegistry"]
        );
    }

    // ------------------------------------------------------------------
    // SHOULD bucket — never affects pass/fail, one fixture per field.
    // ------------------------------------------------------------------

    #[test]
    fn each_should_top_level_field_missing_individually_still_passes_and_names_exactly_that_gap() {
        for field in SHOULD_TOP_LEVEL_FIELDS {
            let mut doc = complete_document();
            doc.as_object_mut().unwrap().remove(field);
            let r = conformant(&input(doc), SPEC_COMMIT, t());
            assert_eq!(
                r.status,
                CheckStatus::Pass,
                "missing SHOULD field {field} must never fail rung 4"
            );
            assert_eq!(
                strs(&r.evidence, "should_gaps"),
                vec![field],
                "removing {field} should name exactly that should_gap"
            );
        }
    }

    #[test]
    fn a_null_should_field_counts_as_a_gap_not_present() {
        let mut doc = complete_document();
        doc["name"] = serde_json::Value::Null;
        let r = conformant(&input(doc), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(strs(&r.evidence, "should_gaps"), vec!["name"]);
    }

    #[test]
    fn services_absent_vs_empty_are_recorded_as_distinct_should_gaps() {
        let mut absent = complete_document();
        absent.as_object_mut().unwrap().remove("services");
        let r_absent = conformant(&input(absent), SPEC_COMMIT, t());
        assert_eq!(r_absent.status, CheckStatus::Pass);
        assert_eq!(r_absent.evidence["services_status"], "absent");
        assert_eq!(strs(&r_absent.evidence, "should_gaps"), vec!["services"]);

        let mut empty = complete_document();
        empty["services"] = json!([]);
        let r_empty = conformant(&input(empty), SPEC_COMMIT, t());
        assert_eq!(r_empty.status, CheckStatus::Pass);
        assert_eq!(r_empty.evidence["services_status"], "empty");
        assert_eq!(
            strs(&r_empty.evidence, "should_gaps"),
            vec!["services_empty"]
        );

        // The two labels never appear on the same document, and each run
        // independently confirms they are not silently merged into one.
        assert_ne!(
            r_absent.evidence["should_gaps"],
            r_empty.evidence["should_gaps"]
        );
    }

    #[test]
    fn services_present_and_non_empty_is_no_gap_at_all() {
        let r = conformant(&input(complete_document()), SPEC_COMMIT, t());
        assert_eq!(r.evidence["services_status"], "present");
        assert!(!strs(&r.evidence, "should_gaps").contains(&"services".to_string()));
        assert!(!strs(&r.evidence, "should_gaps").contains(&"services_empty".to_string()));
    }

    #[test]
    fn a_service_entry_missing_version_is_one_aggregated_should_gap() {
        let mut doc = complete_document();
        doc["services"] = json!([
            { "name": "web", "endpoint": "https://web.agentxyz.com/" }
        ]);
        let r = conformant(&input(doc), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(strs(&r.evidence, "should_gaps"), vec!["services[].version"]);
    }

    #[test]
    fn multiple_service_entries_missing_version_still_yield_one_gap_label() {
        let mut doc = complete_document();
        doc["services"] = json!([
            { "name": "web", "endpoint": "https://web.agentxyz.com/" },
            { "name": "mcp", "endpoint": "https://mcp.agent.eth/" },
        ]);
        let r = conformant(&input(doc), SPEC_COMMIT, t());
        assert_eq!(
            strs(&r.evidence, "should_gaps"),
            vec!["services[].version"],
            "many entries missing version is still ONE completeness gap, not N"
        );
    }

    #[test]
    fn legacy_endpoints_entries_are_also_checked_for_version() {
        let mut doc = complete_document();
        doc.as_object_mut().unwrap().remove("services");
        doc["endpoints"] = json!([{ "name": "web", "endpoint": "https://web.agentxyz.com/" }]);
        let r = conformant(&input(doc), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(strs(&r.evidence, "should_gaps"), vec!["services[].version"]);
    }

    #[test]
    fn registrations_present_but_empty_is_the_same_should_gap_as_absent() {
        let mut doc = complete_document();
        doc["registrations"] = json!([]);
        let r = conformant(&input(doc), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(strs(&r.evidence, "should_gaps"), vec!["registrations"]);
        assert_eq!(r.evidence["registrations_checked"], 0);
    }

    // ------------------------------------------------------------------
    // MAY bucket — purely informational, one fixture per field.
    // ------------------------------------------------------------------

    #[test]
    fn each_may_field_missing_individually_still_passes_and_names_exactly_that_gap() {
        for field in MAY_FIELDS {
            let mut doc = complete_document();
            doc.as_object_mut().unwrap().remove(field);
            let r = conformant(&input(doc), SPEC_COMMIT, t());
            assert_eq!(r.status, CheckStatus::Pass);
            assert_eq!(strs(&r.evidence, "may_gaps"), vec![field]);
            assert!(strs(&r.evidence, "should_gaps").is_empty());
        }
    }

    #[test]
    fn updated_at_is_never_checked_in_any_bucket() {
        // The work order's MAY table names `updatedAt`, but the pinned spec
        // never defines it (see module doc). It must not silently appear as
        // a gap in either direction: present or absent, it has no effect.
        let mut with_it = complete_document();
        with_it["updatedAt"] = json!(1_753_000_000);
        let r_with = conformant(&input(with_it), SPEC_COMMIT, t());
        assert!(strs(&r_with.evidence, "may_gaps").is_empty());

        let without_it = complete_document(); // never had it
        let r_without = conformant(&input(without_it), SPEC_COMMIT, t());
        assert!(!strs(&r_without.evidence, "may_gaps").contains(&"updatedAt".to_string()));
    }

    #[test]
    fn a_document_missing_everything_still_passes_with_zero_must_violations() {
        // The uncomfortable finding, exercised directly: an empty object has
        // no `registrations` key, so the only MUST is never triggered.
        let r = conformant(&input(json!({})), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert!(strs(&r.evidence, "must_violations").is_empty());
        // But it is maximally incomplete under SHOULD: type, name,
        // description, image, services (absent), registrations (absent).
        let gaps = strs(&r.evidence, "should_gaps");
        for expected in [
            "type",
            "name",
            "description",
            "image",
            "services",
            "registrations",
        ] {
            assert!(
                gaps.contains(&expected.to_string()),
                "expected {expected} gap"
            );
        }
        assert_eq!(gaps.len(), 6);
    }

    // ------------------------------------------------------------------
    // Non-object documents fail neither by crashing nor by MUST — see
    // module doc's explanation of why this is the honest consequence.
    // ------------------------------------------------------------------

    #[test]
    fn a_json_array_document_passes_with_zero_must_violations_but_full_should_gaps() {
        let r = conformant(&input(json!([1, 2, 3])), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert!(strs(&r.evidence, "must_violations").is_empty());
        assert!(!strs(&r.evidence, "should_gaps").is_empty());
    }

    #[test]
    fn a_json_string_document_does_not_panic() {
        let r = conformant(&input(json!("just a string")), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Pass);
    }

    #[test]
    fn a_json_number_document_does_not_panic() {
        let r = conformant(&input(json!(42)), SPEC_COMMIT, t());
        assert_eq!(r.status, CheckStatus::Pass);
    }

    #[test]
    fn evidence_carries_the_spec_commit_on_both_pass_and_fail() {
        let pass = conformant(&input(complete_document()), SPEC_COMMIT, t());
        assert_eq!(pass.evidence["spec_commit"], SPEC_COMMIT);

        let mut doc = complete_document();
        doc["registrations"] = json!([{ "agentId": 1 }]);
        let fail = conformant(&input(doc), SPEC_COMMIT, t());
        assert_eq!(fail.status, CheckStatus::Fail);
        assert_eq!(fail.evidence["spec_commit"], SPEC_COMMIT);
    }

    // --- P0 FIX 1: `services` / `endpoints` alias, carried forward -------

    #[test]
    fn only_legacy_endpoints_field_passes_with_legacy_flag_set() {
        let mut doc = complete_document();
        doc.as_object_mut().unwrap().remove("services");
        doc["endpoints"] =
            json!([{ "name": "web", "endpoint": "https://web.agentxyz.com/", "version": "1.0" }]);

        let r = conformant(&input(doc), SPEC_COMMIT, t());

        assert_eq!(r.status, CheckStatus::Pass);
        assert!(!strs(&r.evidence, "should_gaps").contains(&"services".to_string()));
        assert_eq!(r.evidence["legacy_endpoints_field"], true);
        assert_eq!(r.evidence["both_fields_present"], false);
        assert_eq!(r.evidence["services_field_source"], "endpoints");
        assert_eq!(r.evidence["services_status"], "present");
    }

    #[test]
    fn both_services_and_endpoints_present_uses_services_and_flags_both() {
        let mut doc = complete_document();
        doc["endpoints"] = json!([{ "name": "legacy", "endpoint": "https://legacy.example/" }]);

        let r = conformant(&input(doc), SPEC_COMMIT, t());

        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["both_fields_present"], true);
        assert_eq!(r.evidence["legacy_endpoints_field"], false);
        assert_eq!(r.evidence["services_field_source"], "services");
    }

    #[test]
    fn neither_services_nor_endpoints_is_a_should_gap_never_a_must_violation() {
        let mut doc = complete_document();
        doc.as_object_mut().unwrap().remove("services");

        let r = conformant(&input(doc), SPEC_COMMIT, t());

        assert_eq!(r.status, CheckStatus::Pass);
        assert!(strs(&r.evidence, "must_violations").is_empty());
        assert_eq!(strs(&r.evidence, "should_gaps"), vec!["services"]);
        assert_eq!(r.evidence["services_field_source"], "neither");
        assert_eq!(r.evidence["services_status"], "absent");
    }

    #[test]
    fn a_null_endpoints_value_does_not_count_as_the_legacy_alias() {
        let mut doc = complete_document();
        doc.as_object_mut().unwrap().remove("services");
        doc["endpoints"] = serde_json::Value::Null;

        let r = conformant(&input(doc), SPEC_COMMIT, t());

        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["services_field_source"], "neither");
        assert_eq!(r.evidence["services_status"], "absent");
    }
}
