//! What changed between two seller sweeps.
//!
//! METHODOLOGY §10.6. The weekly unit of change ships with the instrument
//! rather than four migrations later, and it inherits every lesson §9 paid
//! for in the registration census — including the two incidents, so that
//! this instrument never has to have its own version of them.
//!
//! # The rule this module exists to hold
//!
//! **A transition into or out of `refused`, `error` or `unprobed` is not
//! churn.** All three are excluded from the two headline series by rule:
//!
//! * `refused` — the origin declined us (rate limit, auth challenge, a
//!   robots.txt that said no). Not the seller going away. This is the
//!   2026-08-06 rule: 19,962 self-inflicted 429s were briefly published as
//!   agents that had stopped resolving.
//! * `error` — OUR probe failed. This is the 2026-08-18 rule: a night of
//!   checker-side timeouts was briefly published as 4,479 agents going dark.
//! * `unprobed` — we chose not to ask (over cap, unpriced, out of scope,
//!   past a host budget). A seller we did not ask about cannot have changed
//!   in our eyes. **This one is new here**, because this instrument is the
//!   first to have a word for "we did not ask", and it is exactly the shape
//!   of the other two: a fact about us, not about the population.
//!
//! Every transition is still recorded in `flips`, and the excluded volumes
//! are totalled so each exclusion is visible in the same place as the
//! numbers that benefit from it.
//!
//! # The confounds a seller delta has that a census delta does not
//!
//! A seller population lives in other people's catalogs, so two more things
//! can move under it:
//!
//! * **The catalog list changed.** A seller that vanishes because its only
//!   catalog was dropped is a method change, not churn.
//! * **The rungs attempted changed.** A sweep that ran the shopper and one
//!   that did not are not comparable on delivery, and a rate that "appears"
//!   because a later sweep asked a question the earlier one skipped is
//!   method, not world.
//!
//! Both travel on the delta beside the checker version, and any surface
//! rendering it must say so — the same rule §9 states for `method_changed`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::identity::SellerId;

/// Every `(seller, rung) -> status` one sweep recorded.
pub type RungStatuses = HashMap<(SellerId, i16), String>;

/// The statuses whose transitions never count as churn. See the module doc.
pub const NOT_CHURN: [&str; 3] = ["refused", "error", "unprobed"];

/// One (rung, from, to) transition and how many sellers made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flip {
    pub rung: i16,
    pub from: String,
    pub to: String,
    pub sellers: i64,
}

/// Everything a seller delta row needs that is derived from the two sweeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SellerDelta {
    pub sellers_before: usize,
    pub sellers_after: usize,
    /// In the newer sweep, absent from the older.
    pub appeared: usize,
    /// In the older sweep, absent from the newer. A seller "disappears" when
    /// no catalog lists it any more — which is a fact about the catalogs as
    /// much as about the seller, and why the catalog-list confound matters.
    pub disappeared: usize,
    /// Rung 2 moved to `pass` from a status that was a real not-pass.
    pub came_back: i64,
    /// Rung 2 moved from `pass` to a real not-pass. The number nobody else
    /// can produce, and therefore the number that must not lie.
    pub went_dark: i64,
    /// The volumes the two series above exclude, by kind, so no exclusion is
    /// silent. Additive: a transition is counted under exactly one.
    pub excluded_refused: i64,
    pub excluded_error: i64,
    pub excluded_unprobed: i64,
    /// Every transition, sorted, so two computations produce byte-identical
    /// output and a diff of a stored row means the data changed.
    pub flips: Vec<Flip>,
}

/// Compare two sweeps.
///
/// A seller present in only one of them contributes to
/// `appeared`/`disappeared` and to nothing else, and a rung with a row on
/// only one side is not a flip — "we did not ask" is not a change in the
/// world. Both rules are §9's, inherited deliberately.
pub fn compute(before: &RungStatuses, after: &RungStatuses) -> SellerDelta {
    let sellers_before: BTreeSet<&SellerId> = before.keys().map(|(s, _)| s).collect();
    let sellers_after: BTreeSet<&SellerId> = after.keys().map(|(s, _)| s).collect();

    let mut counted: BTreeMap<(i16, &str, &str), i64> = BTreeMap::new();
    for ((seller, rung), new_status) in after {
        let Some(old_status) = before.get(&(seller.clone(), *rung)) else {
            continue;
        };
        if old_status == new_status {
            continue;
        }
        *counted
            .entry((*rung, old_status.as_str(), new_status.as_str()))
            .or_insert(0) += 1;
    }

    // Rung 2 carries the published series, derived from the same transitions
    // as the table beneath it so the two cannot disagree.
    let came_back: i64 = counted
        .iter()
        .filter(|((rung, from, to), _)| {
            *rung == 2 && *from != "pass" && *to == "pass" && !NOT_CHURN.contains(from)
        })
        .map(|(_, n)| *n)
        .sum();
    let went_dark: i64 = counted
        .iter()
        .filter(|((rung, from, to), _)| {
            *rung == 2 && *from == "pass" && *to != "pass" && !NOT_CHURN.contains(to)
        })
        .map(|(_, n)| *n)
        .sum();

    // The excluded volumes, by kind. A transition touching two excluded
    // statuses (error → refused) is counted once, under the first that
    // matches NOT_CHURN's order, so the three totals stay additive.
    let mut excluded = [0i64; 3];
    for ((rung, from, to), n) in &counted {
        if *rung != 2 {
            continue;
        }
        if let Some(i) = NOT_CHURN.iter().position(|s| s == from || s == to) {
            excluded[i] += n;
        }
    }

    let flips: Vec<Flip> = counted
        .into_iter()
        .map(|((rung, from, to), sellers)| Flip {
            rung,
            from: from.to_string(),
            to: to.to_string(),
            sellers,
        })
        .collect();

    SellerDelta {
        sellers_before: sellers_before.len(),
        sellers_after: sellers_after.len(),
        appeared: sellers_after.difference(&sellers_before).count(),
        disappeared: sellers_before.difference(&sellers_after).count(),
        came_back,
        went_dark,
        excluded_refused: excluded[0],
        excluded_error: excluded[1],
        excluded_unprobed: excluded[2],
        flips,
    }
}

/// Whether two sweeps are comparable on the world, or whether some of what
/// moved is method.
///
/// Served precomputed for the same reason §9 serves `method_changed`: no
/// consumer should have to remember to compare four things before rendering
/// a number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodConfound {
    pub checker_changed: bool,
    /// The catalog list differs. A seller that vanished because its only
    /// catalog was dropped is a method change, not churn.
    pub catalogs_changed: bool,
    /// The sweeps asked different questions. A delivery rate that "appears"
    /// because a later sweep ran the shopper is method, not world.
    pub rungs_changed: bool,
}

impl MethodConfound {
    /// Any of the three. The one boolean a renderer needs.
    pub fn changed(&self) -> bool {
        self.checker_changed || self.catalogs_changed || self.rungs_changed
    }

    pub fn between(
        checker_before: &str,
        checker_after: &str,
        catalogs_before: &[String],
        catalogs_after: &[String],
        rungs_before: &[i16],
        rungs_after: &[i16],
    ) -> Self {
        let set = |v: &[String]| -> BTreeSet<String> { v.iter().cloned().collect() };
        let rungs = |v: &[i16]| -> BTreeSet<i16> { v.iter().copied().collect() };
        Self {
            checker_changed: checker_before != checker_after,
            catalogs_changed: set(catalogs_before) != set(catalogs_after),
            rungs_changed: rungs(rungs_before) != rungs(rungs_after),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Network;

    fn seller(n: u8) -> SellerId {
        SellerId::new(
            &format!("0x{:040x}", n),
            &format!("https://s{n}.example/x"),
            Network::Evm,
        )
        .unwrap()
    }

    fn statuses(entries: &[(u8, i16, &str)]) -> RungStatuses {
        entries
            .iter()
            .map(|(n, rung, s)| ((seller(*n), *rung), s.to_string()))
            .collect()
    }

    fn flip_of(d: &SellerDelta, rung: i16, from: &str, to: &str) -> i64 {
        d.flips
            .iter()
            .find(|f| f.rung == rung && f.from == from && f.to == to)
            .map(|f| f.sellers)
            .unwrap_or(0)
    }

    #[test]
    fn a_real_stop_is_counted() {
        let d = compute(&statuses(&[(1, 2, "pass")]), &statuses(&[(1, 2, "fail")]));
        assert_eq!(d.went_dark, 1);
        assert_eq!(d.came_back, 0);
    }

    #[test]
    fn a_rate_limit_is_not_a_seller_going_dark() {
        // The 2026-08-06 rule, inherited rather than rediscovered.
        let d = compute(
            &statuses(&[(1, 2, "pass")]),
            &statuses(&[(1, 2, "refused")]),
        );
        assert_eq!(d.went_dark, 0);
        assert_eq!(d.excluded_refused, 1);
        assert_eq!(flip_of(&d, 2, "pass", "refused"), 1, "evidence is kept");
    }

    #[test]
    fn our_own_probe_failing_is_not_a_seller_going_dark() {
        // The 2026-08-18 rule, likewise.
        let d = compute(&statuses(&[(1, 2, "pass")]), &statuses(&[(1, 2, "error")]));
        assert_eq!(d.went_dark, 0);
        assert_eq!(d.excluded_error, 1);
    }

    #[test]
    fn a_seller_we_chose_not_to_ask_about_cannot_have_changed() {
        // The word this instrument adds, and the same shape as the other
        // two: a fact about us, not about the population.
        let d = compute(
            &statuses(&[(1, 2, "pass")]),
            &statuses(&[(1, 2, "unprobed")]),
        );
        assert_eq!(d.went_dark, 0);
        assert_eq!(d.excluded_unprobed, 1);
        // ...and coming back from unprobed is not a return, either.
        let back = compute(
            &statuses(&[(1, 2, "unprobed")]),
            &statuses(&[(1, 2, "pass")]),
        );
        assert_eq!(back.came_back, 0);
        assert_eq!(back.excluded_unprobed, 1);
    }

    #[test]
    fn the_three_excluded_volumes_are_additive() {
        // Every excluded transition is counted under exactly one kind, so
        // the three totals can be published side by side without
        // double-counting.
        let before = statuses(&[
            (1, 2, "pass"),
            (2, 2, "pass"),
            (3, 2, "pass"),
            (4, 2, "pass"),
        ]);
        let after = statuses(&[
            (1, 2, "refused"),
            (2, 2, "error"),
            (3, 2, "unprobed"),
            (4, 2, "fail"),
        ]);
        let d = compute(&before, &after);
        assert_eq!(d.went_dark, 1, "only the definitive answer");
        assert_eq!(
            d.excluded_refused + d.excluded_error + d.excluded_unprobed,
            3
        );
        assert_eq!(d.excluded_refused, 1);
        assert_eq!(d.excluded_error, 1);
        assert_eq!(d.excluded_unprobed, 1);
    }

    #[test]
    fn an_arrival_is_a_population_change_and_never_a_flip() {
        let d = compute(
            &statuses(&[(1, 2, "pass")]),
            &statuses(&[(1, 2, "pass"), (2, 2, "fail")]),
        );
        assert_eq!(d.appeared, 1);
        assert_eq!(d.disappeared, 0);
        assert_eq!(d.went_dark, 0);
        assert!(d.flips.is_empty());
    }

    #[test]
    fn a_rung_asked_on_only_one_side_is_never_a_flip() {
        // "We did not ask" is not a change in the world — which is exactly
        // what a sweep that skipped the shopper would otherwise manufacture.
        let d = compute(
            &statuses(&[(1, 2, "pass")]),
            &statuses(&[(1, 2, "pass"), (1, 4, "pass")]),
        );
        assert!(d.flips.is_empty(), "{:?}", d.flips);
    }

    #[test]
    fn flips_are_ordered_deterministically() {
        let d = compute(
            &statuses(&[(1, 2, "pass"), (2, 7, "pass"), (3, 2, "error")]),
            &statuses(&[(1, 2, "fail"), (2, 7, "fail"), (3, 2, "pass")]),
        );
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
                (7, "pass", "fail")
            ]
        );
    }

    #[test]
    fn a_sweep_that_asked_different_questions_is_flagged_as_method() {
        // The confound this instrument has that the census does not: sweep 1
        // skips the shopper, sweep 2 runs it, and the delivery rate
        // "appears". That is method, not world.
        let c = MethodConfound::between(
            "0.1.0",
            "0.1.0",
            &["bazaar".into()],
            &["bazaar".into()],
            &[1, 2, 3, 7],
            &[1, 2, 3, 4, 7],
        );
        assert!(c.rungs_changed);
        assert!(c.changed());
        assert!(!c.checker_changed && !c.catalogs_changed);
    }

    #[test]
    fn a_dropped_catalog_is_method_not_churn() {
        let c = MethodConfound::between(
            "0.1.0",
            "0.1.0",
            &["bazaar".into(), "x402scan".into()],
            &["bazaar".into()],
            &[1, 2, 3],
            &[1, 2, 3],
        );
        assert!(c.catalogs_changed);
        assert!(c.changed());
    }

    #[test]
    fn two_sweeps_of_the_same_method_are_comparable() {
        let c = MethodConfound::between(
            "0.1.0",
            "0.1.0",
            &["bazaar".into()],
            &["bazaar".into()],
            &[1, 2, 3, 7],
            &[7, 3, 2, 1], // order is not method
        );
        assert!(!c.changed());
    }
}
