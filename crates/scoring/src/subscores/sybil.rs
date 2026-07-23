//! Sybil penalty — the multiplier that discounts manufactured reputation.
//!
//! A "Sybil attack" is one operator wearing many masks: dozens of agents that
//! look independent but are really one puppeteer. This is THE central threat to
//! any on-chain reputation system, and defeating it is Ledgerscope's headline
//! claim. Everything else in the score is additive and positive; this one is
//! different — it's a **penalty applied as a multiplier** to the final result:
//!
//! ```text
//! final = raw · (1 − sybil_penalty)
//! ```
//!
//! So `sybil_penalty = 0.0` leaves a score untouched, while `1.0` annihilates it
//! entirely. That multiplicative shape is deliberate: no amount of good payment,
//! liveness, age, or reputation can rescue an agent that is clearly part of a
//! bot farm.
//!
//! The heavy lifting — actually *detecting* the clusters — happens in the
//! `enricher` crate (`enricher/src/clustering.rs`), because it needs graph data
//! across ALL agents and thus real I/O. By the time we get here, that work is
//! already distilled into the [`ClusterInfo`] on the agent. This function just
//! turns those cluster signals into a `[0, 1]` penalty.
//!
//! [`ClusterInfo`]: crate::model::ClusterInfo

use crate::model::AgentView;
use super::clamp01;

/// Compute the Sybil penalty in `[0, 1]` (0 = no penalty, 1 = score wiped out).
pub(crate) fn sybil_penalty(agent: &AgentView) -> f64 {
    // The enricher already gave us two distilled signals on `agent.cluster`:
    //   * `suspicion`   — how coordinated the cluster looks, in [0, 1]
    //   * `cluster_size`— how many agents are in it (1 = effectively alone)
    //
    // Suggested recipe:
    //
    //     let c = &agent.cluster;
    //
    //     // A lone agent (size <= 1) can't be part of a ring — no penalty.
    //     if c.cluster_size <= 1 {
    //         return 0.0;
    //     }
    //
    //     // Bigger clusters are more damning, but with diminishing effect, so a
    //     // size-3 pair-plus-one isn't treated the same as a 200-strong farm:
    //     let size_factor = 1.0 - 1.0 / (c.cluster_size as f64); // →1 as size grows
    //
    //     // The penalty is the coordination signal scaled by how large the ring
    //     // is. Both must be present: a large but organic-looking group (low
    //     // suspicion) is only mildly penalised.
    //     clamp01(c.suspicion * size_factor)

    let _ = clamp01;
    let _ = agent;
    todo!("map ClusterInfo (suspicion × size_factor) to a [0,1] penalty")
}

#[cfg(test)]
mod tests {
    #[test]
    fn lone_agent_has_no_penalty() {
        todo!("cluster_size = 1 must yield sybil_penalty = 0.0");
    }

    #[test]
    fn tight_large_cluster_is_heavily_penalised() {
        todo!("suspicion ≈ 1.0 with a large cluster_size should approach 1.0");
    }
}
