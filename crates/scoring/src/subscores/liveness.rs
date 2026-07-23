//! Liveness sub-score — does the agent actually *exist and work*?
//!
//! A surprising amount of on-chain "reputation" belongs to agents whose endpoint
//! is dead, parked, or never served a valid agent-card at all. An agent you
//! cannot reach cannot be doing real work, so liveness is a cheap, hard-to-fake
//! reality check. The enricher probes each endpoint repeatedly over time; here we
//! turn that probe history into a `[0, 1]` number.

use crate::model::AgentView;
use super::clamp01;

/// Compute the liveness sub-score in `[0, 1]`.
pub(crate) fn liveness_score(agent: &AgentView) -> f64 {
    // Never probed → we have no evidence it's alive, so treat it as not-alive.
    // This guard also protects the division below from a divide-by-zero.
    if agent.probe_count == 0 {
        return 0.0;
    }

    // The `as f64` casts are mandatory: `probe_successes` and `probe_count` are
    // integers, and integer division truncates (3 / 4 == 0). Rust never silently
    // converts number types for you — you write the cast, and the intent is
    // explicit. Casting to floating point first gives us the real 0.75.
    let rate = agent.probe_successes as f64 / agent.probe_count as f64;

    // `clamp01` is belt-and-braces: `successes` should never exceed `count`, but
    // if a data bug ever made it so, we cap at 1.0 rather than serve 1.2.
    clamp01(rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AgentView;

    #[test]
    fn zero_probes_scores_zero() {
        let mut a = AgentView::sample();
        a.probe_count = 0;
        a.probe_successes = 0;
        assert_eq!(liveness_score(&a), 0.0);
    }

    #[test]
    fn success_rate_is_the_score() {
        let mut a = AgentView::sample();
        a.probe_count = 4;
        a.probe_successes = 3;
        assert_eq!(liveness_score(&a), 0.75);
    }
}
