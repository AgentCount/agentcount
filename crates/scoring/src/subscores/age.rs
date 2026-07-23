//! Age sub-score — reward *sustained* presence, punish flash-in-the-pan farms.
//!
//! Manufactured agents share a tell: they are born, they accumulate a burst of
//! ratings and payments in a very short window, and that's it. Genuinely useful
//! agents tend to show a long history with activity spread across many days.
//!
//! So this score rewards two things together:
//!   * **span** — how long ago the agent first appeared, and
//!   * **spread** — how many distinct days it was actually active over that span.
//! Rewarding span alone would be gameable (register early, do nothing); rewarding
//! spread guards against the "500 ratings in one afternoon" pattern.

use crate::model::AgentView;
use super::clamp01;

/// Compute the age sub-score in `[0, 1]`.
///
/// Purity note: this function takes the "now" it needs *from the data*
/// (`agent.last_activity`) rather than calling `Utc::now()`. That keeps the
/// function deterministic — the same `AgentView` always yields the same score,
/// which is essential for a reproducible, auditable methodology. If you decide
/// you truly need wall-clock "now", pass it in as a parameter; never read the
/// clock inside a scoring function.
pub(crate) fn age_score(agent: &AgentView) -> f64 {
    // Suggested recipe:
    //
    //     // How many days the agent has existed (span), saturating so ancient
    //     // agents don't run away with it:
    //     let span_days = (agent.last_activity - agent.first_seen).num_days().max(0) as f64;
    //     let span = 1.0 - (-span_days / SPAN_K).exp();      // SPAN_K ≈ 90, say
    //
    //     // What fraction of that span had actual activity (spread):
    //     let spread = if span_days > 0.0 {
    //         clamp01(agent.active_days as f64 / span_days)
    //     } else {
    //         0.0
    //     };
    //
    //     // Multiply, so BOTH must be healthy — a long-but-idle agent (high span,
    //     // near-zero spread) is correctly dragged down, and vice versa:
    //     clamp01(span * spread)
    //
    // `chrono`'s `Duration` (returned by subtracting two `DateTime`s) is where
    // `.num_days()` comes from. `.max(0)` guards against clock skew making the
    // span negative.

    let _ = clamp01;
    let _ = agent;
    todo!("combine saturating span with activity spread, then clamp01")
}

#[cfg(test)]
mod tests {
    #[test]
    fn long_but_idle_agent_scores_low() {
        todo!("an agent with a huge span but active_days = 1 should score low, \
               proving spread — not just age — is what we reward");
    }
}
