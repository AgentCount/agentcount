//! Payment sub-score — reward *real economic activity*, resist faked volume.
//!
//! The gaming attack we defend against: an operator sends money in a circle
//! between a few wallets they control, manufacturing huge "volume" that means
//! nothing. Our defence is to reward **counterparty diversity** far more than
//! raw value, and to **log-scale** value so a whale isn't linearly better than a
//! genuinely-used small agent.
//!
//! Intuition for the shape we want:
//!   * 1000 payments from 1 wallet    → low score  (looks like self-dealing)
//!   * 1000 payments from 800 wallets → high score (looks like real demand)

use crate::model::AgentView;
use super::clamp01;

/// Controls how fast the diversity term saturates. With `K = 25`, going from a
/// handful of counterparties to a few dozen moves the score a lot, while
/// 900 → 910 barely registers — which matches how we actually reason about it.
const DIVERSITY_K: f64 = 25.0;

/// Divisor that maps log-scaled value into `[0, 1]`. `ln(1 + 1_000_000) ≈ 13.8`,
/// so with `VALUE_NORM = 14.0` roughly a million units of value approaches the
/// top of the value term. Tune to your normalized-value unit.
const VALUE_NORM: f64 = 14.0;

/// How much diversity dominates the blend. Diversity is the harder signal to
/// fake, so it carries the bulk of the weight; value only tops it up.
const DIVERSITY_SHARE: f64 = 0.75;

/// Compute the payment sub-score in `[0, 1]`.
pub(crate) fn payment_score(agent: &AgentView) -> f64 {
    // 1. Diversity term. `1 - e^(-n/K)` is a saturating curve: 0 at n=0, rising
    //    steeply at first, then flattening toward 1. Perfect for "more distinct
    //    counterparties is better, with diminishing returns".
    let n = agent.distinct_counterparties as f64;
    let diversity = 1.0 - (-n / DIVERSITY_K).exp();

    // 2. Value term, log-scaled. `ln(1 + x)` is 0 at x=0 and grows slowly, so a
    //    whale can't linearly dominate. `.max(0.0)` guards against a stray
    //    negative value producing `ln` of something < 1 (or a NaN).
    let value = clamp01((1.0 + agent.total_payment_value.max(0.0)).ln() / VALUE_NORM);

    // 3. Blend, diversity-dominant, and clamp so no input can escape [0, 1].
    clamp01(DIVERSITY_SHARE * diversity + (1.0 - DIVERSITY_SHARE) * value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AgentView;

    /// The core anti-self-dealing property: holding value fixed, MORE distinct
    /// counterparties must never DECREASE the score. This one test guards the
    /// entire intent of the sub-score.
    #[test]
    fn more_diversity_never_hurts() {
        let mut few = AgentView::sample();
        few.distinct_counterparties = 3;

        let mut many = few.clone();
        many.distinct_counterparties = 300; // only this field differs

        assert!(payment_score(&many) >= payment_score(&few));
    }

    /// A brand-new agent with no economic activity at all scores 0.
    #[test]
    fn no_activity_scores_zero() {
        let mut a = AgentView::sample();
        a.distinct_counterparties = 0;
        a.total_payment_value = 0.0;
        assert_eq!(payment_score(&a), 0.0);
    }
}
