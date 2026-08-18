//! What changed between two runs on one chain — the computation, without the
//! database.
//!
//! Split out of the `delta` binary on 2026-08-06 so that two things could
//! happen: the churn rule below could be tested directly, and
//! `backfill-refused` could recompute an existing delta without a second copy
//! of this arithmetic. A delta is a published series; two implementations of it
//! would eventually be two answers.
//!
//! # The rule this module exists to hold
//!
//! `stopped_resolving` is the number nobody else can produce, and therefore the
//! number that must not lie. **A transition into or out of `refused` or
//! `error` is not churn**, and is excluded from
//! `stopped_resolving`/`newly_resolving` by rule, not by anybody remembering
//! to filter it.
//!
//! The 2026-08 census is why `refused` is here. It reported 19,983 BSC agents
//! as having stopped resolving. 19,962 of those were HTTP 429 — 19,658 from a
//! single host — which is traffic we generated, at a host we chose the
//! concurrency for. Excluding 429/503, that chain lost **10** agents. "19,983
//! agents went dark" is the most quotable wrong thing this project could
//! publish, and the delta table must not be able to say it again.
//!
//! `refused` means the origin is demonstrably there and declined us (see
//! `checks::CheckStatus::Refused`). An agent moving `pass → refused` has not
//! been shown to have stopped resolving; it has been shown to have rate-limited
//! us, or asked us for credentials, or told us through `robots.txt` not to ask.
//! An agent moving `refused → pass` has not started resolving either — we
//! simply got an answer this week that we were declined last week. Both are
//! facts about the conversation, not about the population.
//!
//! The 2026-08-17 Base sweep is why `error` joined it (2026-08-18). That sweep
//! ran ~17 hours through a degraded network — 45-second RPC timeouts from the
//! sweep's own environment, the whole night — and its delta booked 4,479 Base
//! agents as `stopped_resolving`, of which **4,477 were `pass → error`**.
//! `error` is defined as OUR failure — "a timeout in our prober, an RPC
//! error. Never the agent's" (`checks::CheckStatus::Error`) — so counting its
//! transitions as churn published a checker-side outage as agents going dark:
//! the 19,983 mistake again, one status over. The same argument holds in both
//! directions, exactly as it does for `refused`: `error → pass` is our prober
//! recovering, not an agent returning. The cost is stated plainly — a server
//! that vanishes outright surfaces as `error` too (rung 2 books DNS and
//! connection failures as ours, because one observer cannot tell a dead
//! server from its own unreachability), so this series now undercounts true
//! deaths rather than ever overcounting them. That is the direction of error
//! this project chooses everywhere.
//!
//! **Every transition is still recorded in `flips`, including these.** The
//! exclusion is on the two headline series only. Deleting the evidence would
//! make the rate limit invisible, which is the same failure in the other
//! direction — the whole finding above came from being able to count the 429s.

use std::collections::{HashMap, HashSet};

/// Every `(agent_id, rung) -> status` one run recorded.
pub type RungStatuses = HashMap<(u64, i16), String>;

/// The statuses whose transitions never count as churn. See the module doc.
pub const NOT_CHURN: [&str; 2] = ["refused", "error"];

/// One (rung, from, to) transition and how many agents made it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flip {
    pub rung: i16,
    pub from: String,
    pub to: String,
    pub agents: i64,
}

/// Everything a `run_deltas` row needs that is derived from the two runs'
/// results — provenance and ids belong to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaCounts {
    pub agents_before: usize,
    pub agents_after: usize,
    pub newly_registered: usize,
    pub disappeared: usize,
    pub newly_resolving: i64,
    pub stopped_resolving: i64,
    /// Every transition, in a deterministic order, so two runs of this
    /// computation produce byte-identical JSON and a diff of the stored row
    /// means the data changed.
    pub flips: Vec<Flip>,
}

impl DeltaCounts {
    /// `flips` as the JSON shape `run_deltas.flips` stores.
    pub fn flips_json(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.flips
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "rung": f.rung, "from": f.from, "to": f.to, "agents": f.agents,
                    })
                })
                .collect(),
        )
    }
}

/// Compare two runs' results.
///
/// Two things are deliberately NOT transitions, and both are absences rather
/// than movements:
///
/// * An agent present in one run and not the other is a population change.
///   Folding arrivals into "changed status" would make the largest flip in
///   every sweep be new registrations, which says nothing.
/// * A rung with a row in one run and none in the other. That is how rung 6
///   shipping would otherwise have appeared: 27,956 agents "flipping" from
///   nothing to a status, which is a fact about this project rather than about
///   them. Both sides must have a row for the same rung before a flip counts.
pub fn compute(before: &RungStatuses, after: &RungStatuses) -> DeltaCounts {
    let agents_after: HashSet<u64> = after.keys().map(|(a, _)| *a).collect();
    let agents_before: HashSet<u64> = before.keys().map(|(a, _)| *a).collect();

    let mut counted: HashMap<(i16, &str, &str), i64> = HashMap::new();
    for ((agent, rung), new_status) in after {
        let Some(old_status) = before.get(&(*agent, *rung)) else {
            continue;
        };
        if old_status == new_status {
            continue;
        }
        *counted
            .entry((*rung, old_status.as_str(), new_status.as_str()))
            .or_insert(0) += 1;
    }

    // Rung 2 called out on its own, because it carries a published series.
    // Derived from the same transitions rather than counted separately, so the
    // headline number and the table underneath it cannot disagree — and
    // filtered by the one rule this module exists for.
    let newly_resolving: i64 = counted
        .iter()
        .filter(|((rung, from, to), _)| {
            *rung == 2 && *from != "pass" && *to == "pass" && !NOT_CHURN.contains(from)
        })
        .map(|(_, n)| *n)
        .sum();
    let stopped_resolving: i64 = counted
        .iter()
        .filter(|((rung, from, to), _)| {
            *rung == 2 && *from == "pass" && *to != "pass" && !NOT_CHURN.contains(to)
        })
        .map(|(_, n)| *n)
        .sum();

    let mut flips: Vec<Flip> = counted
        .into_iter()
        .map(|((rung, from, to), agents)| Flip {
            rung,
            from: from.to_string(),
            to: to.to_string(),
            agents,
        })
        .collect();
    flips.sort_by(|a, b| (a.rung, &a.from, &a.to).cmp(&(b.rung, &b.from, &b.to)));

    DeltaCounts {
        agents_before: agents_before.len(),
        agents_after: agents_after.len(),
        newly_registered: agents_after.difference(&agents_before).count(),
        disappeared: agents_before.difference(&agents_after).count(),
        newly_resolving,
        stopped_resolving,
        flips,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` agents at `rung`, all with the same status, starting at `from_id`.
    fn run(entries: &[(u64, i16, &str)]) -> RungStatuses {
        entries
            .iter()
            .map(|(a, r, s)| ((*a, *r), s.to_string()))
            .collect()
    }

    fn flip_of(d: &DeltaCounts, rung: i16, from: &str, to: &str) -> i64 {
        d.flips
            .iter()
            .find(|f| f.rung == rung && f.from == from && f.to == to)
            .map(|f| f.agents)
            .unwrap_or(0)
    }

    #[test]
    fn a_real_stop_is_still_counted() {
        let before = run(&[(1, 2, "pass"), (2, 2, "pass")]);
        let after = run(&[(1, 2, "fail"), (2, 2, "pass")]);
        let d = compute(&before, &after);
        assert_eq!(d.stopped_resolving, 1);
        assert_eq!(d.newly_resolving, 0);
    }

    #[test]
    fn a_pass_to_refused_transition_is_not_a_stop() {
        // THE case. A rate limit is not a death.
        let before = run(&[(1, 2, "pass")]);
        let after = run(&[(1, 2, "refused")]);
        let d = compute(&before, &after);
        assert_eq!(
            d.stopped_resolving, 0,
            "a 429 must never be publishable as an agent that stopped resolving"
        );
        // ...and the transition is still visible, because the evidence is how
        // the rate limit was found in the first place.
        assert_eq!(flip_of(&d, 2, "pass", "refused"), 1);
    }

    #[test]
    fn a_refused_to_pass_transition_is_not_a_start() {
        let before = run(&[(1, 2, "refused")]);
        let after = run(&[(1, 2, "pass")]);
        let d = compute(&before, &after);
        assert_eq!(
            d.newly_resolving, 0,
            "getting through this week is not the agent having come back"
        );
        assert_eq!(flip_of(&d, 2, "refused", "pass"), 1);
    }

    #[test]
    fn a_pass_to_error_transition_is_not_a_stop() {
        // THE 2026-08-17 case. Our prober timing out is not a death.
        let before = run(&[(1, 2, "pass")]);
        let after = run(&[(1, 2, "error")]);
        let d = compute(&before, &after);
        assert_eq!(
            d.stopped_resolving, 0,
            "our own timeout must never be publishable as an agent that stopped resolving"
        );
        assert_eq!(flip_of(&d, 2, "pass", "error"), 1);
    }

    #[test]
    fn an_error_to_pass_transition_is_not_a_start() {
        let before = run(&[(1, 2, "error")]);
        let after = run(&[(1, 2, "pass")]);
        let d = compute(&before, &after);
        assert_eq!(
            d.newly_resolving, 0,
            "our prober recovering is not the agent having come back"
        );
        assert_eq!(flip_of(&d, 2, "error", "pass"), 1);
    }

    #[test]
    fn the_2026_08_17_base_shape_reports_two_not_four_and_a_half_thousand() {
        // The incident that extended the rule to `error`: the Base sweep ran
        // ~17 hours through a degraded network (45s RPC timeouts throughout),
        // and 4,477 agents moved pass → error while 2 moved pass → fail.
        // "4,479 Base agents went dark" would have been the 19,983 mistake
        // again, one status over.
        let mut before = Vec::new();
        let mut after = Vec::new();
        for id in 0..4_477u64 {
            before.push((id, 2i16, "pass"));
            after.push((id, 2i16, "error"));
        }
        for id in 100_000..100_002u64 {
            before.push((id, 2, "pass"));
            after.push((id, 2, "fail"));
        }
        let d = compute(&run(&before), &run(&after));
        assert_eq!(d.stopped_resolving, 2, "only the definitive answers count");
        assert_eq!(flip_of(&d, 2, "pass", "error"), 4_477);
    }

    #[test]
    fn refused_to_fail_and_fail_to_refused_are_both_excluded() {
        // Neither direction touches `pass`, so neither belongs in either
        // series — but the exclusion must hold whichever end `refused` is on.
        let before = run(&[(1, 2, "refused"), (2, 2, "fail")]);
        let after = run(&[(1, 2, "fail"), (2, 2, "refused")]);
        let d = compute(&before, &after);
        assert_eq!(d.stopped_resolving, 0);
        assert_eq!(d.newly_resolving, 0);
        assert_eq!(d.flips.len(), 2);
    }

    #[test]
    fn the_2026_08_bsc_shape_reports_ten_not_nineteen_thousand() {
        // The census that motivated all of this, in miniature and to scale:
        // 19,962 agents rate-limited, 11 probe errors of ours, and 10 to
        // `fail` — matching "excluding 429/503, BSC lost 10 agents" exactly,
        // now that `error` is excluded too (2026-08-18; it briefly made this
        // number 21).
        let mut before = Vec::new();
        let mut after = Vec::new();
        for id in 0..19_962u64 {
            before.push((id, 2i16, "pass"));
            after.push((id, 2i16, "refused"));
        }
        for id in 100_000..100_010u64 {
            before.push((id, 2, "pass"));
            after.push((id, 2, "fail"));
        }
        for id in 200_000..200_011u64 {
            before.push((id, 2, "pass"));
            after.push((id, 2, "error"));
        }
        let d = compute(&run(&before), &run(&after));
        assert_eq!(
            d.stopped_resolving, 10,
            "only the agents that actually stopped answering"
        );
        assert_eq!(flip_of(&d, 2, "pass", "refused"), 19_962);
        assert_eq!(flip_of(&d, 2, "pass", "error"), 11);
        assert_eq!(flip_of(&d, 2, "pass", "fail"), 10);
    }

    #[test]
    fn refused_at_another_rung_does_not_touch_the_rung_2_series() {
        // Rung 6 produces `refused` too, and neither headline series is about
        // rung 6.
        let before = run(&[(1, 6, "pass"), (1, 2, "pass")]);
        let after = run(&[(1, 6, "refused"), (1, 2, "pass")]);
        let d = compute(&before, &after);
        assert_eq!(d.stopped_resolving, 0);
        assert_eq!(flip_of(&d, 6, "pass", "refused"), 1);
    }

    #[test]
    fn an_arrival_is_a_population_change_and_never_a_flip() {
        let before = run(&[(1, 2, "pass")]);
        let after = run(&[(1, 2, "pass"), (2, 2, "fail")]);
        let d = compute(&before, &after);
        assert_eq!(d.newly_registered, 1);
        assert_eq!(d.disappeared, 0);
        assert_eq!(d.stopped_resolving, 0);
        assert!(d.flips.is_empty());
    }

    #[test]
    fn a_rung_absent_on_one_side_is_never_a_flip() {
        let before = run(&[(1, 2, "pass")]);
        let after = run(&[(1, 2, "pass"), (1, 6, "pass")]);
        let d = compute(&before, &after);
        assert!(d.flips.is_empty(), "{:?}", d.flips);
        assert_eq!(d.agents_before, 1);
        assert_eq!(d.agents_after, 1);
    }

    #[test]
    fn flips_are_ordered_deterministically() {
        let before = run(&[(1, 2, "pass"), (2, 5, "pass"), (3, 2, "error")]);
        let after = run(&[(1, 2, "fail"), (2, 5, "unclaimed"), (3, 2, "pass")]);
        let d = compute(&before, &after);
        let order: Vec<(i16, &str, &str)> = d
            .flips
            .iter()
            .map(|f| (f.rung, f.from.as_str(), f.to.as_str()))
            .collect();
        assert_eq!(
            order,
            [
                (2, "error", "pass"),
                (2, "pass", "fail"),
                (5, "pass", "unclaimed")
            ]
        );
        assert_eq!(d.flips_json().as_array().unwrap().len(), 3);
        assert_eq!(d.flips_json()[0]["agents"], 1);
    }
}
