//! Ladder semantics: the one place skip-propagation is decided.
//!
//! The rule, and the reason for it: if rung N does not pass, rungs above it
//! are `Skipped` — never `Fail`. You cannot judge the JSON validity of a
//! document you never received, and recording `fail` for a question you never
//! asked is the single easiest way to slander someone by accident.

use serde_json::json;

use crate::model::{CheckResult, CheckStatus};

/// Apply skip-propagation to one agent's results.
///
/// Input may be in any order and may be sparse — a rung we have not
/// implemented simply is not in the list, and MUST NOT be synthesised as
/// `Skipped`: absent means "not checked", which is a different claim.
pub fn run_ladder(mut rungs: Vec<CheckResult>) -> Vec<CheckResult> {
    rungs.sort_by_key(|r| r.rung);

    // Once something stops the ladder, remember what and why — a reader of a
    // skipped rung deserves to know which question went unanswered first.
    let mut stopper: Option<(u8, CheckStatus)> = None;

    for r in rungs.iter_mut() {
        if let Some((rung, status)) = stopper {
            r.status = CheckStatus::Skipped;
            r.evidence = json!({
                "skipped_because_rung": rung,
                "skipped_because_status": status.as_str(),
            });
            continue;
        }
        if r.status != CheckStatus::Pass {
            stopper = Some((r.rung, r.status));
        }
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
            res(7, "independent", CheckStatus::Pass),
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

    /// Same sparse shape, but rung 5 fails: rung 7 (the only rung above the
    /// gap) must be `Skipped` — never `Fail`, and never silently dropped —
    /// even though there is no rung 6 row sitting between them.
    #[test]
    fn a_sparse_ladder_still_propagates_skip_across_the_rung_6_gap() {
        let out = run_ladder(vec![
            res(1, "registered", CheckStatus::Pass),
            res(2, "resolvable", CheckStatus::Pass),
            res(3, "parseable", CheckStatus::Pass),
            res(4, "conformant", CheckStatus::Pass),
            res(5, "bound", CheckStatus::Fail),
            res(7, "independent", CheckStatus::Pass),
        ]);
        assert_eq!(out.len(), 6);
        let rung7 = out.iter().find(|r| r.rung == 7).unwrap();
        assert_eq!(rung7.status, CheckStatus::Skipped);
        assert_eq!(rung7.evidence["skipped_because_rung"], 5);
    }
}
