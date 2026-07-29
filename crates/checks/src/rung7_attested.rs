//! Rung 7 — `attested`: has this agent received at least one Reputation
//! Registry feedback entry, from any client address at all?
//!
//! **P0 FIX 4/5 (2026-07-29) — renamed from `independent`, and ungated.**
//! This rung used to be called `independent` and asked a narrower question:
//! does at least one feedback entry come from an address that is *not* the
//! agent's current owner. That question turned out to be unanswerable in a
//! way that matters — see "Why this rung no longer compares against the
//! owner" below — and the old rung was also gated on rungs 1 through 5 all
//! passing, which meant it only ever ran for ~1,437 of ~60,000 agents (the
//! ones whose *document* happened to resolve, parse, conform, and bind).
//! Reputation feedback lives in the Reputation Registry, keyed by agent id,
//! and is readable for any agent that exists on chain regardless of whether
//! its document ever resolved at all — there was never a real dependency on
//! rungs 2 through 5. See `crates/checks/src/ladder.rs`'s module doc for how
//! the ladder itself now encodes that: this rung's only dependency is rung 1.
//!
//! ## Why this rung no longer compares against the owner
//!
//! The pinned spec is explicit, `spec/ERC8004SPEC.md` line 217: *"The
//! feedback submitter MUST NOT be the agent owner or an approved operator
//! for `agentId`."* That is a contract-level invariant, not a heuristic —
//! feedback from the owner's own address cannot be successfully submitted in
//! the first place. The old `independent` rung computed
//! `authors_equal_to_owner` and `self_feedback_ratio` anyway and reported
//! "zero agents caught writing their own reviews" as if it were a finding.
//! It was not: publishing a measurement whose only possible outcome is
//! restating a rule nobody can break is not a measurement, and it would have
//! been publicly corrected within a day at the full ~60,000-agent scale this
//! fix ungates the rung to. **This rung does not, and cannot, detect
//! self-review — no evidence field, log line, or report copy produced here
//! claims otherwise.** `AttestedInput` no longer even carries the agent's
//! owner address; there is nothing left in this rung's logic to compare it
//! against.
//!
//! What this rung asks instead is the question it can actually answer: did
//! anyone — anyone at all — leave this agent feedback. That is still a real
//! floor: `getClients` returning zero addresses means nobody has vouched for
//! this agent in any capacity, self or otherwise.
//!
//! **This is not a quality signal.** It does not read feedback *values* —
//! whether the score was glowing or damning — only whether any exists. An
//! agent with one scathing review still passes; an agent with zero reviews
//! fails regardless of how good its document is elsewhere in the ladder.
//!
//! ## Sybil and coordination analysis does not belong here
//!
//! Two addresses under common control, one feeding the other glowing
//! feedback, would still pass this rung — the same was true of the old
//! `independent` rung, and remains true here. Detecting that would take
//! clustering inference (funding-linked addresses, deployer-linked
//! addresses, operator-adjacency), which is a fundamentally different kind
//! of claim than a measurement this rung's evidence lets a reader
//! recompute by hand. If this project ever publishes that analysis, it goes
//! in a separately-labelled `signals` block, never folded into a rung
//! result — and it is **not built as part of this fix**. The spec itself
//! flags the underlying problem (line 324): `getSummary` requires a
//! non-empty `clientAddresses` filter precisely because unfiltered feedback
//! is subject to Sybil/spam attacks, and expects reviewer-reputation to
//! emerge off-chain, outside this contract's scope.
//!
//! ## `self_feedback_ratio` and `authors_equal_to_owner` are dropped, not renamed
//!
//! The old evidence carried `authors_equal_to_owner` and a derived
//! `self_feedback_ratio`. Both are gone from this rung's evidence, on
//! purpose, not merely renamed: computing either now would mean silently
//! re-introducing the owner comparison this fix's whole point is to remove
//! from a rung result, and — because owner self-feedback is contract-level
//! impossible — the ratio would still be trivially `0.0` (or `null`) for
//! essentially every agent, restating the same tautology as *evidence*
//! instead of as a verdict. Evidence that can only ever have one value is
//! not evidence; it is decoration. `feedback_count` and `distinct_authors`
//! survive unchanged — both are genuine per-agent measurements a reader can
//! recompute from `getClients`/`getSummary` themselves, and neither implies
//! anything about who those authors are relative to the owner.
//!
//! ## Whose fault is "no registry"?
//!
//! Unchanged from the old rung: a chain without a deployed Reputation
//! Registry makes this rung unanswerable, not failed: `registry_available:
//! false` maps to `Error`, never `Fail`. The agent did nothing wrong — the
//! infrastructure to ask the question doesn't exist on that chain.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde_json::json;

use crate::model::{CheckResult, CheckStatus};

/// What rung 7 needs to know: every distinct address that has left an agent
/// feedback, the total feedback entry count, and whether the chain even has
/// a Reputation Registry to ask. Deliberately carries no owner address —
/// see the module doc's "Why this rung no longer compares against the
/// owner".
#[derive(Debug, Clone)]
pub struct AttestedInput {
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

pub fn attested(input: &AttestedInput, now: DateTime<Utc>) -> CheckResult {
    if !input.registry_available {
        let evidence = json!({
            "reason": "no_reputation_registry",
            "feedback_count": input.feedback_count,
            "distinct_authors": null,
        });
        return CheckResult {
            rung: 7,
            name: "attested",
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
    let distinct_authors = distinct_clients.len() as u64;

    let (status, reason) = if distinct_authors == 0 {
        (CheckStatus::Fail, Some("no_feedback"))
    } else {
        (CheckStatus::Pass, None)
    };

    let mut evidence = json!({
        "feedback_count": input.feedback_count,
        "distinct_authors": distinct_authors,
    });
    if let Some(reason) = reason {
        evidence["reason"] = json!(reason);
    }

    CheckResult {
        rung: 7,
        name: "attested",
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

    fn input(clients: Vec<&str>, feedback_count: u64, registry_available: bool) -> AttestedInput {
        AttestedInput {
            clients: clients.into_iter().map(String::from).collect(),
            feedback_count,
            registry_available,
        }
    }

    #[test]
    fn any_feedback_at_all_passes_regardless_of_who_left_it() {
        // Deliberately named OWNER — this rung does not know, and does not
        // ask, whether a client address matches the agent's owner. A single
        // client, even one that would have been "self-feedback" under the
        // old rung's framing, is enough to pass. See the module doc: owner
        // self-feedback is impossible at the contract level anyway.
        let r = attested(&input(vec![OWNER], 5, true), t());
        assert_eq!(r.rung, 7);
        assert_eq!(r.name, "attested");
        assert_eq!(r.status, CheckStatus::Pass);
        assert!(r.evidence.get("reason").is_none());
        assert_eq!(r.evidence["distinct_authors"], 1);
        assert_eq!(r.evidence["feedback_count"], 5);
        // The dropped fields must not reappear under any name.
        assert!(r.evidence.get("authors_equal_to_owner").is_none());
        assert!(r.evidence.get("self_feedback_ratio").is_none());
    }

    #[test]
    fn several_distinct_authors_pass() {
        let r = attested(&input(vec![OWNER, OTHER, ANOTHER], 12, true), t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert!(r.evidence.get("reason").is_none());
        assert_eq!(r.evidence["distinct_authors"], 3);
        assert_eq!(r.evidence["feedback_count"], 12);
    }

    #[test]
    fn zero_clients_fails_as_no_feedback() {
        let r = attested(&input(vec![], 0, true), t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["reason"], "no_feedback");
        assert_eq!(r.evidence["distinct_authors"], 0);
        assert_eq!(r.evidence["feedback_count"], 0);
    }

    #[test]
    fn registry_unavailable_errors_and_does_not_blame_the_agent() {
        let r = attested(&input(vec![OWNER], 3, false), t());
        assert_eq!(r.status, CheckStatus::Error);
        assert_eq!(r.evidence["reason"], "no_reputation_registry");
        assert!(r.evidence["distinct_authors"].is_null());
    }

    #[test]
    fn registry_unavailable_takes_precedence_even_with_clients_present() {
        // Even if clients happen to include several addresses, an
        // unavailable registry means we never actually asked — Error, not
        // Pass, because the input can't be trusted as a real read.
        let r = attested(&input(vec![OWNER, OTHER], 3, false), t());
        assert_eq!(r.status, CheckStatus::Error);
        assert_eq!(r.evidence["reason"], "no_reputation_registry");
    }

    #[test]
    fn duplicate_client_entries_in_different_case_do_not_inflate_distinct_authors() {
        let r = attested(&input(vec![OTHER, &OTHER.to_uppercase()], 2, true), t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["distinct_authors"], 1);
    }

    #[test]
    fn a_single_mixed_case_client_still_passes() {
        let r = attested(&input(vec![&OTHER.to_uppercase()], 1, true), t());
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["distinct_authors"], 1);
    }
}
