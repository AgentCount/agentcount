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
        CheckResult { rung, name, status, evidence: serde_json::json!({}), checked_at: t() }
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
}
