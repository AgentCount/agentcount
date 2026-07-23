//! Payment sub-score — reward *real economic activity*, resist faked volume.
//!
//! The gaming attack we defend against: an operator sends money in a circle
//! between a few wallets they control, manufacturing huge "volume" that means
//! nothing. Our defence is to reward **counterparty diversity** far more than
//! raw value, and to **log-scale** value so a whale isn't linearly better than a
//! genuinely-used small agent.
//!
//! Intuition for the shape we want:
//!   * 1000 payments from 1 wallet   → low score  (looks like self-dealing)
//!   * 1000 payments from 800 wallets → high score (looks like real demand)

use crate::model::AgentView;
use super::clamp01;

/// Compute the payment sub-score in `[0, 1]`.
pub(crate) fn payment_score(agent: &AgentView) -> f64 {
    // Suggested recipe (fill in and tune):
    //
    // 1. Diversity term — the dominant signal. Something saturating, so that
    //    going from 1→10 distinct counterparties matters a lot but 900→910
    //    barely moves the needle:
    //        let diversity = 1.0 - (-(agent.distinct_counterparties as f64) / K).exp();
    //    (K controls how fast it saturates; try K ≈ 25 and eyeball it.)
    //
    // 2. Value term — log-scaled so it can't dominate. `ln(1 + x)` is the usual
    //    trick because it's 0 at x=0 and grows slowly:
    //        let value = (1.0 + agent.total_payment_value).ln() / VALUE_NORM;
    //
    // 3. Blend, weighting diversity higher, then clamp:
    //        clamp01(0.75 * diversity + 0.25 * clamp01(value))
    //
    // Keep `clamp01` on the final result so no input can push you out of range.

    let _ = agent;
    todo!("compute a diversity-dominant, log-scaled payment score, then clamp01")
}

#[cfg(test)]
mod tests {
    // Property worth pinning down once implemented: with total value held fixed,
    // MORE distinct counterparties must never DECREASE the score (monotonicity).
    // That single test guards the entire anti-self-dealing intent.
    #[test]
    fn more_diversity_never_hurts() {
        todo!("build two AgentViews differing only in distinct_counterparties, \
               assert the higher-diversity one scores >= the other");
    }
}
