//! Reputation sub-score — the hardest signal to trust, so we trust it least.
//!
//! On-chain "reputation" is just other agents attesting "this one is good".
//! That is trivially gameable: spin up 50 sock-puppet agents, have them all
//! praise your main agent. So a naive "sum of feedback" score is exactly the
//! *manufactured reputation* the whole project exists to expose.
//!
//! ## Why a plain weighted average is not enough (a subtle bug worth learning)
//!
//! The obvious design is: reputation = weighted average of feedback values,
//! where each attester's weight is its trustworthiness. But look what happens
//! when every attester says the same thing (say, "1.0 — perfect"): the weighted
//! average is 1.0 *regardless of the weights*, because an average of identical
//! values is that value. A ring of worthless sock-puppets all shouting "10/10"
//! would score a perfect 1.0. The weighting did nothing.
//!
//! The fix is to separate two questions:
//!   * **quality**    — *what* do credible sources say? (weighted average of values)
//!   * **confidence** — *how much* credible evidence is there at all? (total weight)
//!
//! and multiply them. A sock-puppet ring has tiny total credible weight → low
//! confidence → low reputation, even if its (fake) quality is 1.0. Independent,
//! established praise has high credible weight → high confidence → high score.
//!
//! ## The two credibility signals we apply per edge
//!
//!   1. **Attester weight** — praise from a long-lived, independent agent counts;
//!      praise from a day-old account that only rates its friends barely counts.
//!      Pre-computed by the enricher (EigenTrust-flavoured) and carried on each edge.
//!   2. **Reciprocity discount** — if A rates B and B rates A, that mutual
//!      back-scratch is a collusion tell and is worth a fraction of a normal edge.

use crate::model::AgentView;
use super::clamp01;

/// A reciprocal (mutual) rating counts for only this fraction of a one-directional
/// one. Low, because mutual rating is a strong collusion signal.
const RECIPROCITY_FACTOR: f64 = 0.3;

/// How much total credible weight is needed before we're "confident". With
/// `K = 3`, a few units of trustworthy attestation give solid confidence, while
/// a whisper of low-weight praise gives almost none. Controls the confidence curve.
const CONFIDENCE_K: f64 = 3.0;

/// Compute the reputation sub-score in `[0, 1]`.
pub(crate) fn reputation_score(agent: &AgentView) -> f64 {
    // Accumulate the total credible weight and the weighted sum of values in one
    // pass. `&agent.incoming_feedback` borrows the vector — we read it without
    // consuming it, so `agent` stays usable afterwards.
    let mut weight_sum = 0.0;
    let mut value_weight_sum = 0.0;

    for edge in &agent.incoming_feedback {
        // Knock down reciprocal edges. `if cond { a } else { b }` is an
        // *expression* in Rust — it evaluates to a value we assign directly.
        let reciprocity = if edge.is_reciprocal {
            RECIPROCITY_FACTOR
        } else {
            1.0
        };
        // Never let a stray negative weight subtract credibility.
        let w = edge.attester_weight.max(0.0) * reciprocity;
        weight_sum += w;
        value_weight_sum += w * edge.raw_value;
    }

    // No *credible* feedback at all → no reputation. This is the right answer,
    // not an error: plenty of legitimate agents simply have no ratings yet.
    if weight_sum == 0.0 {
        return 0.0;
    }

    // quality: what credible sources say, on average, in [0, 1].
    let quality = clamp01(value_weight_sum / weight_sum);

    // confidence: how much credible evidence exists, saturating toward 1.
    let confidence = 1.0 - (-weight_sum / CONFIDENCE_K).exp();

    clamp01(quality * confidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentView, FeedbackEdge};

    // Small constructor helpers keep the tests readable.
    fn edge(weight: f64, value: f64, reciprocal: bool) -> FeedbackEdge {
        FeedbackEdge {
            from_agent_id: 99,
            raw_value: value,
            attester_weight: weight,
            is_reciprocal: reciprocal,
        }
    }

    /// THE anti-gaming property. Both agents receive three "perfect" (1.0)
    /// ratings, so a naive average would tie them at 1.0. But one set is a
    /// reciprocal ring of low-weight sock-puppets and the other is independent,
    /// high-weight praise. The ring must score strictly lower.
    #[test]
    fn reciprocal_ring_scores_lower_than_independent_praise() {
        let mut ring = AgentView::sample();
        ring.incoming_feedback = vec![
            edge(0.1, 1.0, true),
            edge(0.1, 1.0, true),
            edge(0.1, 1.0, true),
        ];

        let mut independent = AgentView::sample();
        independent.incoming_feedback = vec![
            edge(1.0, 1.0, false),
            edge(1.0, 1.0, false),
            edge(1.0, 1.0, false),
        ];

        assert!(reputation_score(&ring) < reputation_score(&independent));
    }

    /// An agent with no feedback scores 0 rather than panicking on divide-by-zero.
    #[test]
    fn no_feedback_scores_zero() {
        let mut a = AgentView::sample();
        a.incoming_feedback = vec![];
        assert_eq!(reputation_score(&a), 0.0);
    }
}
