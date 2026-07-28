//! Rung 7 — `independent`: does at least one Reputation Registry feedback
//! entry come from an address that is **not** the agent's current owner?
//!
//! Every rung below this one can be satisfied entirely by the agent's own
//! operator: they control the registration document, the URI it resolves
//! to, and the on-chain claims it makes about itself. Reputation feedback is
//! the first signal in this ladder that, in principle, comes from someone
//! else — but only in principle, because nothing stops the owner from
//! calling `giveFeedback` (or an equivalent write) about their own agent
//! from their own address. Rung 7 catches exactly that: an agent whose only
//! "reputation" is a review it wrote about itself.
//!
//! **This is not a quality signal.** It does not read feedback *values* —
//! whether the score was glowing or damning — only *authorship*. An agent
//! could have one scathing independent review and still pass; an agent
//! could have glowing self-praise and still fail. The question is narrowly
//! "did anyone else show up", not "did they like what they saw".
//!
//! ## `self_feedback_ratio` is evidence, not a score
//!
//! This product's whole premise is that no rung produces a number that
//! stands in for a verdict — see the crate doc comment. `self_feedback_ratio`
//! sits close enough to that line that it earns its own justification: it is
//! permitted because it describes **one measurement** (what fraction of this
//! agent's distinct feedback authors is the owner), not a number that
//! combines multiple rungs or agents into a ranking. It is per-rung evidence
//! a reader can recompute from `authors_equal_to_owner` and
//! `distinct_authors` alone — exactly like `registrations_seen` in rung 5 or
//! `body_bytes` in rung 3, just expressed as a fraction instead of a count.
//!
//! Defined precisely: `authors_equal_to_owner / distinct_authors`, and
//! **`null` when `distinct_authors` is `0`** — never `0.0`. A measured zero
//! (every one of several authors happens not to be the owner — impossible
//! here since `authors_equal_to_owner` is 0 or 1, but the principle
//! generalises) and a measurement that could not be taken (no authors to
//! measure at all) are different claims; collapsing "we couldn't measure
//! this" into "we measured zero" is exactly the kind of silent wrongness
//! this project exists to avoid.
//!
//! ## Whose fault is "no registry"?
//!
//! A chain without a deployed Reputation Registry makes this rung
//! unanswerable, not failed: `registry_available: false` maps to `Error`,
//! never `Fail`. The agent did nothing wrong — the infrastructure to ask the
//! question doesn't exist on that chain. Conflating "we couldn't check" with
//! "they failed the check" would blame the agent for a gap that is ours.
//!
//! ## Address comparison
//!
//! Owner and client addresses are expected to already be lowercase (both
//! [`chain::registry::AgentSnapshot::owner`] and
//! [`chain::reputation::FeedbackReads::clients`] normalise once, at the
//! read). This rung does not re-normalise — see the self-review note in the
//! task brief this was written from — but every comparison here uses
//! [`str::eq_ignore_ascii_case`] rather than `==`, so a caller that (against
//! that contract) passes mixed-case input is still compared correctly rather
//! than silently misjudged.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde_json::json;

use crate::model::{CheckResult, CheckStatus};

/// What rung 7 needs to know: the agent's current owner, every distinct
/// address that has left it feedback, the total feedback entry count, and
/// whether the chain even has a Reputation Registry to ask.
#[derive(Debug, Clone)]
pub struct IndependentInput {
    /// Lowercase hex, from `ownerOf` at the pinned block.
    pub owner: String,
    /// Lowercase hex, from `getClients` at the pinned block. May contain
    /// duplicates or non-lowercase entries in principle (defended against
    /// below via case-insensitive, de-duplicating comparison); in practice
    /// the chain crate returns a deduplicated, lowercase list.
    pub clients: Vec<String>,
    pub feedback_count: u64,
    /// `false` when the chain has no Reputation Registry at all — a
    /// property of the chain, resolved before this rung runs, never a
    /// property of this agent.
    pub registry_available: bool,
}

pub fn independent(input: &IndependentInput, now: DateTime<Utc>) -> CheckResult {
    if !input.registry_available {
        let evidence = json!({
            "reason": "no_reputation_registry",
            "feedback_count": input.feedback_count,
            "distinct_authors": null,
            "authors_equal_to_owner": null,
            "self_feedback_ratio": null,
        });
        return CheckResult {
            rung: 7,
            name: "independent",
            status: CheckStatus::Error,
            evidence,
            checked_at: now,
        };
    }

    // De-duplicated, case-insensitively: two entries differing only in case
    // are the same author, and a caller that (against the documented
    // contract above) hands in duplicates must not inflate the count.
    let distinct_clients: HashSet<String> =
        input.clients.iter().map(|c| c.to_lowercase()).collect();
    let owner_lower = input.owner.to_lowercase();

    let distinct_authors = distinct_clients.len() as u64;
    let authors_equal_to_owner = distinct_clients
        .iter()
        .filter(|c| **c == owner_lower)
        .count() as u64;

    // `null`, never `0.0` — see the module doc comment on why those are
    // different claims.
    let self_feedback_ratio = if distinct_authors == 0 {
        serde_json::Value::Null
    } else {
        json!(authors_equal_to_owner as f64 / distinct_authors as f64)
    };

    let (status, reason) = if distinct_authors == 0 {
        (CheckStatus::Fail, Some("no_feedback"))
    } else if distinct_clients.iter().any(|c| c != &owner_lower) {
        (CheckStatus::Pass, None)
    } else {
        (CheckStatus::Fail, Some("only_self_feedback"))
    };

    let mut evidence = json!({
        "feedback_count": input.feedback_count,
        "distinct_authors": distinct_authors,
        "authors_equal_to_owner": authors_equal_to_owner,
        "self_feedback_ratio": self_feedback_ratio,
    });
    if let Some(reason) = reason {
        evidence["reason"] = json!(reason);
    }

    CheckResult {
        rung: 7,
        name: "independent",
        status,
        evidence,
        checked_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    const OWNER: &str = "0x742d35cc6634c0532925a3b844bc9e7595f6bed1";
    const OTHER: &str = "0x0000000000000000000000000000000000cafe";
    const ANOTHER: &str = "0x0000000000000000000000000000000000beef";

    fn input(
        clients: Vec<&str>,
        feedback_count: u64,
        registry_available: bool,
    ) -> IndependentInput {
        IndependentInput {
            owner: OWNER.to_string(),
            clients: clients.into_iter().map(String::from).collect(),
            feedback_count,
            registry_available,
        }
    }

    #[test]
    fn owner_only_feedback_fails_as_only_self_feedback() {
        let r = independent(&input(vec![OWNER], 5, true), t());
        assert_eq!(r.rung, 7);
        assert_eq!(r.name, "independent");
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["reason"], "only_self_feedback");
        assert_eq!(r.evidence["distinct_authors"], 1);
        assert_eq!(r.evidence["authors_equal_to_owner"], 1);
        assert_eq!(r.evidence["self_feedback_ratio"], 1.0);
        assert_eq!(r.evidence["feedback_count"], 5);
    }

    #[test]
    fn one_independent_author_among_several_self_authored_passes() {
        // Owner is one of several distinct authors; at least one client is
        // not the owner, so this passes even though the owner also left
        // feedback (possibly many times, reflected in feedback_count).
        let r = independent(&input(vec![OWNER, OTHER, ANOTHER], 12, true), t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert!(r.evidence.get("reason").is_none());
        assert_eq!(r.evidence["distinct_authors"], 3);
        assert_eq!(r.evidence["authors_equal_to_owner"], 1);
        assert_eq!(r.evidence["self_feedback_ratio"], 1.0 / 3.0);
        assert_eq!(r.evidence["feedback_count"], 12);
    }

    #[test]
    fn no_independent_and_no_self_feedback_at_all_fails_as_no_feedback() {
        let r = independent(&input(vec![], 0, true), t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["reason"], "no_feedback");
        assert_eq!(r.evidence["distinct_authors"], 0);
        assert_eq!(r.evidence["authors_equal_to_owner"], 0);
    }

    #[test]
    fn zero_distinct_authors_yields_a_null_ratio_not_a_division_by_zero_or_zero_point_zero() {
        let r = independent(&input(vec![], 0, true), t());
        assert!(
            r.evidence["self_feedback_ratio"].is_null(),
            "expected null, got {:?}",
            r.evidence["self_feedback_ratio"]
        );
        assert_ne!(r.evidence["self_feedback_ratio"], 0.0);
    }

    #[test]
    fn registry_unavailable_errors_and_does_not_blame_the_agent() {
        let r = independent(&input(vec![OWNER], 3, false), t());
        assert_eq!(r.status, CheckStatus::Error);
        assert_eq!(r.evidence["reason"], "no_reputation_registry");
    }

    #[test]
    fn registry_unavailable_takes_precedence_even_with_independent_looking_clients() {
        // Even if clients happen to include a non-owner address, an
        // unavailable registry means we never actually asked — Error, not
        // Pass, because the input can't be trusted as a real read.
        let r = independent(&input(vec![OWNER, OTHER], 3, false), t());
        assert_eq!(r.status, CheckStatus::Error);
        assert_eq!(r.evidence["reason"], "no_reputation_registry");
    }

    #[test]
    fn mixed_case_owner_and_client_addresses_are_compared_case_insensitively() {
        let mixed_owner_input = IndependentInput {
            owner: "0x742D35CC6634C0532925A3B844BC9E7595F6BED1".to_string(),
            clients: vec!["0x742d35cc6634c0532925a3b844bc9e7595f6bed1".to_string()],
            feedback_count: 2,
            registry_available: true,
        };
        let r = independent(&mixed_owner_input, t());
        assert_eq!(
            r.status,
            CheckStatus::Fail,
            "same address in different case must still be recognised as the owner"
        );
        assert_eq!(r.evidence["reason"], "only_self_feedback");
        assert_eq!(r.evidence["authors_equal_to_owner"], 1);
    }

    #[test]
    fn mixed_case_independent_client_still_passes() {
        let mixed_case_input = IndependentInput {
            owner: OWNER.to_string(),
            clients: vec![OTHER.to_uppercase()],
            feedback_count: 1,
            registry_available: true,
        };
        let r = independent(&mixed_case_input, t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["authors_equal_to_owner"], 0);
        assert_eq!(r.evidence["self_feedback_ratio"], 0.0);
    }

    #[test]
    fn duplicate_client_entries_in_different_case_do_not_inflate_distinct_authors() {
        let dup_input = IndependentInput {
            owner: OWNER.to_string(),
            clients: vec![OTHER.to_string(), OTHER.to_uppercase()],
            feedback_count: 2,
            registry_available: true,
        };
        let r = independent(&dup_input, t());
        assert_eq!(r.evidence["distinct_authors"], 1);
    }
}
