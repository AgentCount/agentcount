//! Age sub-score — reward *sustained* presence, punish flash-in-the-pan farms.
//!
//! Manufactured agents share a tell: they are born, they accumulate a burst of
//! ratings and payments in a very short window, and that's it. Genuinely useful
//! agents tend to show a long history with activity spread across many days.
//!
//! So this score rewards two things *together*: **span** (how long the agent has
//! existed) and **spread** (what fraction of that span it was actually active
//! over). Rewarding span alone would be gameable (register early, do nothing);
//! rewarding spread guards against the "500 ratings in one afternoon" pattern.
//! We multiply them, so BOTH must be healthy — a long-but-idle agent is correctly
//! dragged down, and so is a busy-but-brand-new one.

use crate::model::AgentView;
use super::clamp01;

/// Controls how fast the span term saturates, in days. With `K = 90`, an agent
/// around a quarter old already earns most of the span credit.
const SPAN_K: f64 = 90.0;

/// Compute the age sub-score in `[0, 1]`.
///
/// Purity note: this function takes the "now" it needs *from the data*
/// (`agent.last_activity`) rather than calling `Utc::now()`. That keeps the
/// function deterministic — the same `AgentView` always yields the same score,
/// essential for a reproducible, auditable methodology. If you ever truly need
/// wall-clock "now", pass it in as a parameter; never read the clock in here.
pub(crate) fn age_score(agent: &AgentView) -> f64 {
    // Days between first sighting and last activity. Subtracting two chrono
    // `DateTime`s yields a `Duration`; `.num_days()` truncates to whole days.
    // `.max(0)` guards against clock skew making the span negative.
    let span_days = (agent.last_activity - agent.first_seen).num_days().max(0) as f64;

    // Span: saturating, so ancient agents don't run away with the score.
    let span = 1.0 - (-span_days / SPAN_K).exp();

    // Spread: what fraction of the span had real activity. Guard the division;
    // a zero-day span (registered and active the same day) has no meaningful
    // spread yet. `clamp01` handles the case where `active_days` slightly exceeds
    // the truncated `span_days`.
    let spread = if span_days > 0.0 {
        clamp01(agent.active_days as f64 / span_days)
    } else {
        0.0
    };

    // Multiply: both signals must be present for a high age score.
    clamp01(span * spread)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AgentView;

    /// The key property: age is about *sustained* activity, not mere age. An
    /// agent that has existed for years but was active on a single day must score
    /// far lower than a younger agent active across many days.
    #[test]
    fn long_but_idle_scores_lower_than_shorter_but_active() {
        let mut idle = AgentView::sample();
        idle.first_seen = chrono::DateTime::from_timestamp(1_500_000_000, 0).unwrap();
        idle.last_activity = idle.first_seen + chrono::Duration::days(3650); // ~10 years
        idle.active_days = 1; // ...but active exactly once

        let active = AgentView::sample(); // 180-day span, 90 active days

        assert!(age_score(&idle) < age_score(&active));
        assert!(age_score(&idle) < 0.05, "a one-day-active agent should score tiny");
    }

    /// A same-day agent (zero span) has no track record yet → 0.
    #[test]
    fn zero_span_scores_zero() {
        let mut a = AgentView::sample();
        a.last_activity = a.first_seen; // span of 0 days
        a.active_days = 1;
        assert_eq!(age_score(&a), 0.0);
    }
}
