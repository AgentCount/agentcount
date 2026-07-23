//! Reputation sub-score — the hardest signal to trust, so we trust it least.
//!
//! On-chain "reputation" is just other agents attesting "this one is good".
//! That is trivially gameable: spin up 50 sock-puppet agents, have them all
//! praise your main agent. So a naive "sum of feedback" score is exactly the
//! *manufactured reputation* the whole project exists to expose.
//!
//! Two defences, both EigenTrust-flavoured (trust flows from already-trusted
//! sources, not from anonymous crowds):
//!
//!   1. **Weight each attester by its own trustworthiness.** Praise from a
//!      long-lived, independent agent counts; praise from a day-old account that
//!      only ever rates its friends barely counts. We receive that per-attester
//!      weight pre-computed on each [`FeedbackEdge`].
//!
//!   2. **Discount reciprocity.** If A rates B and B rates A, that mutual
//!      back-scratch is a collusion tell and is worth less than one-directional
//!      feedback from a disinterested party.

use crate::model::AgentView;
use super::clamp01;

/// Compute the reputation sub-score in `[0, 1]`.
pub(crate) fn reputation_score(agent: &AgentView) -> f64 {
    // `agent.incoming_feedback` is a `Vec<FeedbackEdge>` — a growable list.
    // Iterating it is a nice first taste of Rust iterators.
    //
    // Suggested recipe:
    //
    //     // A weighted average of feedback values, where each edge's weight is
    //     // the attester's trust, knocked down further if the edge is reciprocal.
    //     let mut weight_sum = 0.0;
    //     let mut value_sum  = 0.0;
    //     for edge in &agent.incoming_feedback {
    //         let reciprocity_factor = if edge.is_reciprocal { 0.3 } else { 1.0 };
    //         let w = edge.attester_weight * reciprocity_factor;
    //         weight_sum += w;
    //         value_sum  += w * edge.raw_value;
    //     }
    //     if weight_sum == 0.0 {
    //         return 0.0; // no *credible* feedback → no reputation, not a crash
    //     }
    //     clamp01(value_sum / weight_sum)
    //
    // Note the `&agent.incoming_feedback` borrow in the `for` loop: we read the
    // vector without consuming it, so `agent` stays usable afterwards.

    let _ = clamp01; // used in the sketch above
    let _ = agent;
    todo!("weighted average of incoming feedback, discounting reciprocal edges")
}

#[cfg(test)]
mod tests {
    // Key anti-gaming property: a ring of agents that only rate each other
    // (all edges reciprocal, all attesters low-weight) should score far lower
    // than the same praise coming from independent, one-directional sources.
    #[test]
    fn reciprocal_ring_scores_lower_than_independent_praise() {
        todo!("construct a reciprocal-ring AgentView and an independent-praise \
               AgentView with identical raw_values; assert ring < independent");
    }
}
