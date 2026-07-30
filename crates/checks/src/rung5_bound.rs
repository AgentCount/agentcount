//! Rung 5 — `bound`: does the document name the agent id, registry, and
//! chain we actually fetched it from?
//!
//! Rungs 1 through 4 only ever look *inward* at the document: is it
//! reachable, does it parse, does it carry the fields the spec requires.
//! None of them ask whether the document's claims line up with the on-chain
//! record we resolved it from in the first place. That is exactly the gap a
//! copy-pasted registration slips through: clone someone else's off-chain
//! document (or reuse one across agent ids), change nothing, and every rung
//! up to here passes it clean. Rung 5 is the first rung that compares the
//! document against reality rather than judging it in isolation, by
//! checking the `registrations` entries (spec line 104) against the
//! `actual_agent_id` / `actual_chain_id` / `actual_registry` the caller
//! resolved on-chain:
//!
//! ```text
//! "registrations": [
//!   { "agentId": 22,
//!     "agentRegistry": "{namespace}:{chainId}:{identityRegistry}" }  // e.g. eip155:1:0x742...
//! ]
//! ```
//!
//! **Deliberate asymmetry with rung 4.** There, an absent `registrations` is
//! a SHOULD (spec line 123) and never fails the document — rung 4 only
//! checks presence of fields that exist, and `registrations` itself is
//! optional. Here, absence (or an empty array) is **`unclaimed`**, not
//! `fail`: rung 5's entire question is "does the document bind itself to
//! the record we fetched it from", and a document that lists no
//! registrations at all has made no binding claim to check.
//!
//! **`unclaimed` — added 2026-07-29, replacing what used to be a `fail`.**
//! Once P0 FIX 3 reclassified `registrations` as SHOULD, a document can pass
//! rung 4 while declaring no registrations at all — and none of `pass`,
//! `fail`, `skipped`, or `error` honestly describes that case: `pass` would
//! claim a verification that never happened, `fail` would punish a
//! merely-recommended field exactly as hard as a real mismatch, `skipped`
//! would falsely imply an earlier rung failed, and `error` would falsely
//! imply this checker malfunctioned. `CheckStatus::Unclaimed` is the honest
//! fifth word: the agent made no binding claim for this rung to check.
//! `unclaimed` and `fail` remain distinct in evidence and in every published
//! count — collapsing "declined to claim a binding" into "claimed the wrong
//! one" would erase a real distinction (an agent who says nothing about
//! where it's registered is not making the same mistake as one who names
//! the wrong registry). See `CheckStatus::Unclaimed`'s doc comment and
//! `METHODOLOGY.md` §2 for the full reasoning and citation.
//!
//! **Multiple registrations are legal** — an agent may be registered on
//! several chains — so this rung passes if *any* entry matches all three of
//! agent id, chain id, and registry address. The other entries are not
//! failures; they simply describe a different on-chain presence.
//!
//! **A non-array `registrations`, or one that is present but empty, is
//! treated exactly like an absent one** (`unclaimed`): there is nothing to
//! walk, and turning "not shaped like an array" into a panic or a silent
//! zero would hide the same absence of a binding claim.
//!
//! **`agentId` as a JSON string.** The spec's example writes `agentId` as a
//! JSON number, but token ids are `uint256` on-chain and JSON numbers lose
//! precision above 2^53 — a common enough reason for real documents to
//! serialize large ids as strings that this rung treats it as the same
//! claim, not a different one. `"22"` and `22` are both read as the
//! unsigned integer 22 before comparing to `actual_agent_id`. This rung's
//! job is "does the claimed identity match reality", not "is the document's
//! JSON strictly typed" — that second question belongs to rung 4, and rung
//! 4 does not ask it either (its module doc explicitly defers type
//! checking). Refusing to match a plainly-equal id over a serialization
//! convention would misclassify a genuinely bound agent as unbound, which
//! is the opposite of what this rung is for. A string that does not parse
//! as an unsigned integer (`"abc"`) simply does not match anything.
//!
//! **Namespace must be `eip155`**; any other namespace is a mismatch, not
//! an error — CAIP-2 defines other namespaces, but ERC-8004 registries only
//! ever live on `eip155` (EVM) chains, so a different namespace can never
//! be this agent's binding, it can only fail to match it.
//!
//! **A malformed `agentRegistry`** (not exactly three `:`-separated parts,
//! or not a string at all) parses to nothing and simply fails to match —
//! same treatment as a wrong chain id or address, not a distinct error
//! path, and it cannot panic because every extraction goes through
//! `serde_json::Value`'s `Option`-returning accessors.
//!
//! **Evidence records the best-matching entry**, not just the first one.
//! When several registrations exist, the one that agrees with reality on
//! the most of {agent id, namespace, chain id, address} is the most
//! informative near-miss to publish — that is precisely what makes a
//! copy-paste attempt visible to a reader.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::model::{CheckResult, CheckStatus};

/// The parsed document (from rung 3) plus the on-chain facts we fetched it
/// from — the reality this rung checks the document's own claims against.
#[derive(Debug, Clone)]
pub struct BoundInput {
    pub document: serde_json::Value,
    pub actual_agent_id: u64,
    pub actual_chain_id: u64,
    pub actual_registry: String,
}

/// `agentId` read as an unsigned integer regardless of whether the document
/// wrote it as a JSON number or a numeric JSON string — see the module doc's
/// ruling on the string-vs-number question.
fn parsed_agent_id(entry: &Value) -> Option<u64> {
    match entry.get("agentId") {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.parse::<u64>().ok(),
        _ => None,
    }
}

/// `agentRegistry` split into `(namespace, chain_id, address)`. `None` if
/// the field is missing, not a string, or does not split into exactly three
/// `:`-separated parts — a malformed string, handled without panicking.
fn parsed_registry(entry: &Value) -> Option<(String, Option<u64>, String)> {
    let s = entry.get("agentRegistry")?.as_str()?;
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let namespace = parts[0].to_string();
    let chain_id = parts[1].parse::<u64>().ok();
    let address = parts[2].to_string();
    Some((namespace, chain_id, address))
}

/// One registration entry's evaluation against the on-chain facts.
struct EntryEval {
    declared_agent_id: Value,
    declared_registry: Value,
    declared_chain: Value,
    is_match: bool,
    /// How many of {agent id, namespace, chain id, address} agree with
    /// reality — used only to pick the most informative entry to report,
    /// never to gate pass/fail.
    score: u8,
}

fn evaluate_entry(
    entry: &Value,
    actual_agent_id: u64,
    actual_chain_id: u64,
    actual_registry: &str,
) -> EntryEval {
    let declared_agent_id = entry.get("agentId").cloned().unwrap_or(Value::Null);
    let declared_registry = entry.get("agentRegistry").cloned().unwrap_or(Value::Null);

    let agent_id_match = parsed_agent_id(entry) == Some(actual_agent_id);

    let parsed = parsed_registry(entry);
    let namespace_ok = parsed.as_ref().is_some_and(|(ns, _, _)| ns == "eip155");
    let chain_id = parsed.as_ref().and_then(|(_, cid, _)| *cid);
    let chain_match = chain_id == Some(actual_chain_id);
    let address_match = parsed
        .as_ref()
        .is_some_and(|(_, _, addr)| addr.eq_ignore_ascii_case(actual_registry));

    let is_match = agent_id_match && namespace_ok && chain_match && address_match;
    let score = agent_id_match as u8 + namespace_ok as u8 + chain_match as u8 + address_match as u8;

    EntryEval {
        declared_agent_id,
        declared_registry,
        declared_chain: chain_id.map(Value::from).unwrap_or(Value::Null),
        is_match,
        score,
    }
}

pub fn bound(input: &BoundInput, now: DateTime<Utc>) -> CheckResult {
    let entries = input
        .document
        .get("registrations")
        .filter(|v| !v.is_null())
        .and_then(Value::as_array);

    let Some(entries) = entries.filter(|e| !e.is_empty()) else {
        // Absent, non-array, null, or empty: no binding claim to check at
        // all — `Unclaimed`, not `Fail` (2026-07-29 fix; see the module
        // doc's asymmetry note and `CheckStatus::Unclaimed`'s doc comment).
        // `match` is `null`, not `false`: there is no claim to have matched
        // or mismatched, and `false` would read as "checked and wrong"
        // rather than "nothing to check".
        let evidence = json!({
            "reason": "unclaimed",
            "declared_agent_id": null,
            "declared_registry": null,
            "declared_chain": null,
            "match": null,
            "registrations_seen": 0,
        });
        return CheckResult {
            rung: 5,
            name: "bound",
            status: CheckStatus::Unclaimed,
            evidence,
            checked_at: now,
        };
    };

    let evals: Vec<EntryEval> = entries
        .iter()
        .map(|e| {
            evaluate_entry(
                e,
                input.actual_agent_id,
                input.actual_chain_id,
                &input.actual_registry,
            )
        })
        .collect();

    // The best entry: highest score wins; first entry wins ties. Any exact
    // match necessarily has the maximum possible score, so if one exists it
    // is exactly the one reported.
    let best = evals
        .iter()
        .enumerate()
        .max_by_key(|(i, e)| (e.score, std::cmp::Reverse(*i)))
        .map(|(_, e)| e)
        .expect("non-empty entries");

    let status = if best.is_match {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    };

    let evidence = json!({
        "declared_agent_id": best.declared_agent_id,
        "declared_registry": best.declared_registry,
        "declared_chain": best.declared_chain,
        "match": best.is_match,
        "registrations_seen": entries.len(),
    });

    CheckResult {
        rung: 5,
        name: "bound",
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

    const ACTUAL_AGENT_ID: u64 = 22;
    const ACTUAL_CHAIN_ID: u64 = 1;
    const ACTUAL_REGISTRY: &str = "0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1";

    fn input(document: serde_json::Value) -> BoundInput {
        BoundInput {
            document,
            actual_agent_id: ACTUAL_AGENT_ID,
            actual_chain_id: ACTUAL_CHAIN_ID,
            actual_registry: ACTUAL_REGISTRY.to_string(),
        }
    }

    fn doc_with_registrations(registrations: serde_json::Value) -> serde_json::Value {
        json!({ "registrations": registrations })
    }

    #[test]
    fn exact_match_passes() {
        let doc = doc_with_registrations(json!([
            { "agentId": 22, "agentRegistry": "eip155:1:0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1" }
        ]));
        let r = bound(&input(doc), t());
        assert_eq!(r.rung, 5);
        assert_eq!(r.name, "bound");
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["match"], true);
        assert_eq!(r.evidence["declared_agent_id"], 22);
        assert_eq!(r.evidence["declared_chain"], 1);
        assert_eq!(r.evidence["registrations_seen"], 1);
    }

    #[test]
    fn a_matching_entry_among_several_non_matching_ones_passes() {
        let doc = doc_with_registrations(json!([
            { "agentId": 99, "agentRegistry": "eip155:1:0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1" },
            { "agentId": 22, "agentRegistry": "eip155:8453:0x000000000000000000000000000000deadbeef" },
            { "agentId": 22, "agentRegistry": "eip155:1:0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1" },
        ]));
        let r = bound(&input(doc), t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["match"], true);
        assert_eq!(r.evidence["registrations_seen"], 3);
    }

    #[test]
    fn wrong_agent_id_fails() {
        let doc = doc_with_registrations(json!([
            { "agentId": 23, "agentRegistry": "eip155:1:0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1" }
        ]));
        let r = bound(&input(doc), t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["match"], false);
        assert_eq!(r.evidence["declared_agent_id"], 23);
    }

    #[test]
    fn wrong_chain_id_fails() {
        let doc = doc_with_registrations(json!([
            { "agentId": 22, "agentRegistry": "eip155:8453:0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1" }
        ]));
        let r = bound(&input(doc), t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["match"], false);
        assert_eq!(r.evidence["declared_chain"], 8453);
    }

    #[test]
    fn wrong_registry_address_fails() {
        let doc = doc_with_registrations(json!([
            { "agentId": 22, "agentRegistry": "eip155:1:0x0000000000000000000000000000000000dead" }
        ]));
        let r = bound(&input(doc), t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["match"], false);
    }

    #[test]
    fn same_address_different_case_still_matches() {
        let doc = doc_with_registrations(json!([
            { "agentId": 22, "agentRegistry": "eip155:1:0x742D35CC6634C0532925A3B844BC9E7595F6BED1" }
        ]));
        let r = bound(&input(doc), t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["match"], true);
    }

    #[test]
    fn malformed_agent_registry_string_fails_not_panics() {
        let doc = doc_with_registrations(json!([
            { "agentId": 22, "agentRegistry": "not-a-caip-string" }
        ]));
        let r = bound(&input(doc), t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["match"], false);
        assert!(r.evidence["declared_chain"].is_null());
    }

    #[test]
    fn a_non_eip155_namespace_is_a_mismatch_not_an_error() {
        let doc = doc_with_registrations(json!([
            { "agentId": 22, "agentRegistry": "polkadot:1:0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1" }
        ]));
        let r = bound(&input(doc), t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["match"], false);
        // Chain and address both still line up — this is exactly the near-miss
        // a reader should be able to see, not just a generic "no match".
        assert_eq!(r.evidence["declared_chain"], 1);
    }

    /// Deliverable fixture (P0 FIX 4/5 addendum): `registrations` absent →
    /// `unclaimed`.
    #[test]
    fn absent_registrations_is_unclaimed_not_a_fail() {
        let doc = json!({ "name": "myAgent" });
        let r = bound(&input(doc), t());
        assert_eq!(r.status, CheckStatus::Unclaimed);
        assert_eq!(r.evidence["reason"], "unclaimed");
        assert_eq!(r.evidence["registrations_seen"], 0);
        assert!(
            r.evidence["match"].is_null(),
            "no claim to have matched or not"
        );
    }

    /// Deliverable fixture: `registrations` present but an empty array →
    /// `unclaimed`, same reason as absent — see the module doc's asymmetry
    /// note on why this is not a `fail`.
    #[test]
    fn empty_registrations_array_is_unclaimed_not_a_fail() {
        let doc = doc_with_registrations(json!([]));
        let r = bound(&input(doc), t());
        assert_eq!(r.status, CheckStatus::Unclaimed);
        assert_eq!(r.evidence["reason"], "unclaimed");
    }

    #[test]
    fn agent_id_as_a_json_string_still_matches() {
        // See the module doc's ruling: uint256 ids can legitimately be
        // serialized as strings to avoid JS number precision loss, so "22"
        // and 22 are read as the same claimed identity.
        let doc = doc_with_registrations(json!([
            { "agentId": "22", "agentRegistry": "eip155:1:0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1" }
        ]));
        let r = bound(&input(doc), t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["match"], true);
    }

    #[test]
    fn a_registrations_value_that_is_not_an_array_is_unclaimed_not_panics() {
        for bad in [
            json!("eip155:1:0xabc"),
            json!(42),
            json!({"agentId": 22}),
            json!(true),
        ] {
            let doc = doc_with_registrations(bad);
            let r = bound(&input(doc), t());
            assert_eq!(r.status, CheckStatus::Unclaimed);
            assert_eq!(r.evidence["reason"], "unclaimed");
        }
    }

    #[test]
    fn an_array_of_non_object_entries_fails_not_panics() {
        let doc = doc_with_registrations(json!(["not-an-object", 42, null]));
        let r = bound(&input(doc), t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["match"], false);
        assert_eq!(r.evidence["registrations_seen"], 3);
    }

    #[test]
    fn best_matching_entry_is_reported_even_when_it_is_not_first() {
        let doc = doc_with_registrations(json!([
            { "agentId": 1, "agentRegistry": "eip155:1:0x0000000000000000000000000000000000dead" },
            { "agentId": 22, "agentRegistry": "eip155:1:0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1" },
        ]));
        let r = bound(&input(doc), t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["declared_agent_id"], 22);
    }

    #[test]
    fn a_missing_agent_registry_field_in_an_entry_fails_not_panics() {
        let doc = doc_with_registrations(json!([{ "agentId": 22 }]));
        let r = bound(&input(doc), t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert!(r.evidence["declared_registry"].is_null());
    }
}
