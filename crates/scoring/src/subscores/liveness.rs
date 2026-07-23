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
    // Simplest sensible version: the fraction of probes that succeeded *and*
    // returned a valid agent-card.
    //
    //     if agent.probe_count == 0 {
    //         return 0.0; // never successfully probed → treat as not-alive
    //     }
    //     let rate = agent.probe_successes as f64 / agent.probe_count as f64;
    //     clamp01(rate)
    //
    // Note the `as f64` casts: `probe_successes` / `probe_count` are integers,
    // and integer division would truncate (3/4 = 0). Casting to floating point
    // first is required to get 0.75. Rust makes you write the cast explicitly —
    // it never silently converts number types for you.
    //
    // Later refinements to consider:
    //   * weight recent probes more than old ones (uptime *trend*)
    //   * require a minimum probe_count before awarding a high score, so a lucky
    //     single success can't look like perfect uptime.

    let _ = clamp01;
    let _ = agent;
    todo!("return the valid-probe success rate, guarding against divide-by-zero")
}

#[cfg(test)]
mod tests {
    #[test]
    fn zero_probes_scores_zero() {
        todo!("an AgentView with probe_count = 0 must score exactly 0.0");
    }
}
