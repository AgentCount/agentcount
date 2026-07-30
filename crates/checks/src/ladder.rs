//! Ladder semantics: the one place skip-propagation is decided.
//!
//! The rule, and the reason for it: if rung N does not pass, every rung that
//! *depends* on it is `Skipped` — never `Fail`. You cannot judge the JSON
//! validity of a document you never received, and recording `fail` for a
//! question you never asked is the single easiest way to slander someone by
//! accident.
//!
//! **P0 FIX 4/5 (2026-07-29) — three independent tracks, not one chain.**
//! Before this fix, "depends on" meant nothing more than "has a smaller rung
//! number", so a single failure anywhere below rung 7 — even a rung 7 has
//! nothing to do with, like rung 2's HTTP fetch — silently demoted rung 7 to
//! `Skipped`. That was true only by accident of the old gating (the sweeper
//! used to construct a rung-7 result solely for agents that had already
//! passed rungs 1 through 5, so the accident never surfaced), and it stopped
//! being true the moment rung 7 was ungated to run for every agent that
//! passes rung 1. [`depends_on`] now names each rung's REAL, direct
//! dependency, and skip-propagation follows that graph instead of the
//! numeric ordering:
//!
//! - **Document track** (1 → 2 → 3 → 4 → 5): unchanged, a straight chain —
//!   each rung needs the one directly below it to have passed.
//! - **Service track** (6, not yet implemented): depends on rung 4 — it
//!   needs a document that at least conforms enough to declare `services`,
//!   not the full chain down to rung 1 re-litigated, and *not* rung 5 (a
//!   document can decline to bind itself to the chain and still have live
//!   endpoints worth checking).
//! - **Reputation track** (7): depends on rung 1 *alone*. Reputation
//!   feedback lives in a different registry, readable for any agent id that
//!   exists on chain, regardless of whether its document ever resolved,
//!   parsed, conformed, or bound to it. A rung-2 failure must never be able
//!   to skip rung 7 — see the module's test
//!   `rung_7_keeps_its_own_verdict_even_when_the_document_track_fails`.
//!
//! Rung numbers still happen to be a valid topological order for this graph
//! (every dependency in [`depends_on`] points at a strictly smaller number),
//! so sorting ascending and processing once, left to right, is still
//! sufficient — no separate graph traversal needed.

use std::collections::HashMap;

use serde_json::json;

use crate::model::{CheckResult, CheckStatus};

/// The one rung each rung directly depends on, or `None` for a track's own
/// root. This is the entire dependency graph — see the module doc for why it
/// is three short chains, not one long one.
///
/// Rung 6 is listed even though no code constructs a rung-6 `CheckResult`
/// yet (see `crates/checks::lib`'s module doc and `METHODOLOGY.md` §2): the
/// dependency is part of the rung's specification, decided now so this table
/// does not need revisiting the day rung 6 ships.
fn depends_on(rung: u8) -> Option<u8> {
    match rung {
        2 => Some(1),
        3 => Some(2),
        4 => Some(3),
        5 => Some(4),
        6 => Some(4), // service track: needs a document that reached rung 4
        7 => Some(1), // reputation track: needs only that the agent exists
        _ => None,    // rung 1, and any unrecognised rung, is a track root
    }
}

/// Apply skip-propagation to one agent's results.
///
/// Input may be in any order and may be sparse — a rung we have not
/// implemented simply is not in the list, and MUST NOT be synthesised as
/// `Skipped`: absent means "not checked", which is a different claim.
pub fn run_ladder(mut rungs: Vec<CheckResult>) -> Vec<CheckResult> {
    rungs.sort_by_key(|r| r.rung);

    // For every rung NUMBER seen so far, the (rung, status) that stops
    // anything depending on it — its own, if it didn't pass, or whatever it
    // itself inherited from further down its track. `None` (no entry) means
    // "nothing stops a dependent here", which covers both "this rung passed"
    // and "this rung isn't in the input at all" identically — exactly the
    // sparse-ladder behaviour the doc comment above promises.
    //
    // Ascending rung order is a valid processing order for this graph
    // because every `depends_on` target is numerically smaller than its
    // dependent, so a rung's dependency is always resolved (or absent)
    // before the rung itself is reached.
    let mut stopper_by_rung: HashMap<u8, (u8, CheckStatus)> = HashMap::new();

    for r in rungs.iter_mut() {
        let inherited = depends_on(r.rung).and_then(|parent| stopper_by_rung.get(&parent).copied());
        if let Some((stop_rung, stop_status)) = inherited {
            r.status = CheckStatus::Skipped;
            r.evidence = json!({
                "skipped_because_rung": stop_rung,
                "skipped_because_status": stop_status.as_str(),
            });
            // Propagate the ORIGINAL stopper, not "skipped by my direct
            // parent" — a reader three rungs up a track should see which
            // question actually went unanswered first, not a chain of
            // skips pointing at each other.
            stopper_by_rung.insert(r.rung, (stop_rung, stop_status));
        } else if r.status != CheckStatus::Pass {
            stopper_by_rung.insert(r.rung, (r.rung, r.status));
        }
        // Else: passed, and nothing upstream in its own track stops it —
        // record nothing, so nothing depending on this rung is skipped
        // because of it.
    }
    rungs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use chrono::{DateTime, Utc};

    fn t() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    fn res(rung: u8, name: &'static str, status: CheckStatus) -> CheckResult {
        CheckResult {
            rung,
            name,
            status,
            evidence: serde_json::json!({}),
            checked_at: t(),
        }
    }

    #[test]
    fn all_passing_stays_all_passing() {
        let out = run_ladder(vec![
            res(1, "registered", CheckStatus::Pass),
            res(2, "resolvable", CheckStatus::Pass),
            res(3, "parseable", CheckStatus::Pass),
        ]);
        assert!(out.iter().all(|r| r.status == CheckStatus::Pass));
    }

    #[test]
    fn a_failed_rung_skips_everything_above_it_and_never_fails_them() {
        let out = run_ladder(vec![
            res(1, "registered", CheckStatus::Pass),
            res(2, "resolvable", CheckStatus::Fail),
            res(3, "parseable", CheckStatus::Pass),
            res(4, "conformant", CheckStatus::Pass),
        ]);
        assert_eq!(out[1].status, CheckStatus::Fail);
        // You cannot judge the JSON validity of a document you never received.
        assert_eq!(out[2].status, CheckStatus::Skipped);
        assert_eq!(out[3].status, CheckStatus::Skipped);
        assert!(!out.iter().skip(2).any(|r| r.status == CheckStatus::Fail));
    }

    #[test]
    fn our_own_error_also_skips_downstream_rather_than_blaming_the_agent() {
        let out = run_ladder(vec![
            res(1, "registered", CheckStatus::Pass),
            res(2, "resolvable", CheckStatus::Error),
            res(3, "parseable", CheckStatus::Pass),
        ]);
        assert_eq!(out[2].status, CheckStatus::Skipped);
    }

    #[test]
    fn a_skipped_rung_records_which_rung_stopped_it() {
        let out = run_ladder(vec![
            res(1, "registered", CheckStatus::Fail),
            res(2, "resolvable", CheckStatus::Pass),
        ]);
        assert_eq!(out[1].evidence["skipped_because_rung"], 1);
        assert_eq!(out[1].evidence["skipped_because_status"], "fail");
    }

    #[test]
    fn ladder_is_evaluated_in_rung_order_regardless_of_input_order() {
        let out = run_ladder(vec![
            res(3, "parseable", CheckStatus::Pass),
            res(1, "registered", CheckStatus::Fail),
            res(2, "resolvable", CheckStatus::Pass),
        ]);
        assert_eq!(out[0].rung, 1);
        assert_eq!(out[1].status, CheckStatus::Skipped);
        assert_eq!(out[2].status, CheckStatus::Skipped);
    }

    /// The first place this gap occurs in production: rung 6 (`live`) is not
    /// implemented at all (deferred to Day 4) — this is not "rung 6 skipped",
    /// it is "rung 6 was never asked", and the input vector reflects that by
    /// simply never containing a rung-6 element. `run_ladder` must not
    /// invent one to fill the gap, and it must not let the gap confuse
    /// skip-propagation for rung 7, which sits directly above it.
    #[test]
    fn a_sparse_ladder_missing_rung_6_does_not_invent_a_row_for_it() {
        let out = run_ladder(vec![
            res(1, "registered", CheckStatus::Pass),
            res(2, "resolvable", CheckStatus::Pass),
            res(3, "parseable", CheckStatus::Pass),
            res(4, "conformant", CheckStatus::Pass),
            res(5, "bound", CheckStatus::Pass),
            res(7, "attested", CheckStatus::Pass),
        ]);
        // Exactly six rows in, six rows out — no rung 6 materialised.
        assert_eq!(out.len(), 6);
        assert!(
            !out.iter().any(|r| r.rung == 6),
            "rung 6 is absent, not present with any status"
        );
        assert_eq!(
            out.iter().map(|r| r.rung).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 7]
        );
        // All passing (including the gap) stays all passing — the missing
        // rung 6 must not itself trip skip-propagation for rung 7.
        assert!(out.iter().all(|r| r.status == CheckStatus::Pass));
    }

    /// **P0 FIX 4/5.** Before this fix, rung 5 failing here would have
    /// demoted rung 7 to `Skipped` — that was the exact tautology-preserving
    /// bug this fix removes. Rung 7 (`attested`) depends on rung 1 alone, so
    /// a rung-5 failure elsewhere in the document track must leave rung 7's
    /// own verdict untouched, gap or no gap in between.
    #[test]
    fn a_sparse_ladder_does_not_propagate_a_rung_5_failure_to_rung_7() {
        let out = run_ladder(vec![
            res(1, "registered", CheckStatus::Pass),
            res(2, "resolvable", CheckStatus::Pass),
            res(3, "parseable", CheckStatus::Pass),
            res(4, "conformant", CheckStatus::Pass),
            res(5, "bound", CheckStatus::Fail),
            res(7, "attested", CheckStatus::Pass),
        ]);
        assert_eq!(out.len(), 6);
        let rung5 = out.iter().find(|r| r.rung == 5).unwrap();
        assert_eq!(
            rung5.status,
            CheckStatus::Fail,
            "rung 5's own verdict stands"
        );
        let rung7 = out.iter().find(|r| r.rung == 7).unwrap();
        assert_eq!(
            rung7.status,
            CheckStatus::Pass,
            "rung 7 does not depend on rung 5 — see the module doc's P0 FIX 4/5 note"
        );
    }

    /// The deliverable fixture named explicitly by the P0 FIX 4/5 work
    /// order: rung 7 running — and passing — for an agent that failed rung
    /// 2. This is the whole point of ungating rung 7: reputation feedback is
    /// readable on chain whether or not the document ever resolved.
    #[test]
    fn rung_7_keeps_its_own_verdict_even_when_rung_2_fails() {
        let out = run_ladder(vec![
            res(1, "registered", CheckStatus::Pass),
            res(2, "resolvable", CheckStatus::Fail),
            res(7, "attested", CheckStatus::Pass),
        ]);
        let rung2 = out.iter().find(|r| r.rung == 2).unwrap();
        assert_eq!(rung2.status, CheckStatus::Fail);
        let rung7 = out.iter().find(|r| r.rung == 7).unwrap();
        assert_eq!(rung7.status, CheckStatus::Pass);
        assert!(
            rung7.evidence.get("skipped_because_rung").is_none(),
            "a rung-7 result that was never skipped must not carry skip evidence"
        );
    }

    /// The one case rung 7 IS skipped: rung 1 itself failing (the reputation
    /// track's only dependency — see [`depends_on`]).
    #[test]
    fn rung_7_is_skipped_when_rung_1_fails() {
        let out = run_ladder(vec![
            res(1, "registered", CheckStatus::Fail),
            res(7, "attested", CheckStatus::Pass),
        ]);
        let rung7 = out.iter().find(|r| r.rung == 7).unwrap();
        assert_eq!(rung7.status, CheckStatus::Skipped);
        assert_eq!(rung7.evidence["skipped_because_rung"], 1);
    }

    /// The service track's declared (but not yet implemented) dependency:
    /// rung 6 depends on rung 4, not on rung 5. Exercised directly against
    /// [`depends_on`] via a synthetic rung-6 row, since no production code
    /// constructs one yet — this only proves the table entry itself behaves
    /// as documented, ready for the day rung 6 ships.
    #[test]
    fn rung_6_would_depend_on_rung_4_not_rung_5() {
        let out = run_ladder(vec![
            res(1, "registered", CheckStatus::Pass),
            res(2, "resolvable", CheckStatus::Pass),
            res(3, "parseable", CheckStatus::Pass),
            res(4, "conformant", CheckStatus::Fail),
            res(6, "live", CheckStatus::Pass),
        ]);
        let rung6 = out.iter().find(|r| r.rung == 6).unwrap();
        assert_eq!(rung6.status, CheckStatus::Skipped);
        assert_eq!(rung6.evidence["skipped_because_rung"], 4);
    }
}
