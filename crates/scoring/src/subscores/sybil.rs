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
    let c = &agent.cluster;

    // A lone agent (or a "cluster" of one) can't be part of a ring — no penalty.
    // This also stops a stray non-zero `suspicion` on a singleton from doing harm.
    if c.cluster_size <= 1 {
        return 0.0;
    }

    // Bigger clusters are more damning, but with diminishing effect: `1 - 1/size`
    // rises from 0.5 (a pair) toward 1.0 (a large farm), so a suspicious pair
    // isn't punished as hard as a suspicious swarm.
    let size_factor = 1.0 - 1.0 / (c.cluster_size as f64);

    // The penalty needs BOTH signals: a large but organic-looking group (low
    // suspicion) is only mildly penalised, and a tiny group can't be penalised
    // much no matter how suspicious. `.max(0.0)` guards a stray negative suspicion.
    clamp01(c.suspicion.max(0.0) * size_factor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AgentView;

    #[test]
    fn lone_agent_has_no_penalty() {
        let a = AgentView::sample(); // cluster_size = 1 by default
        assert_eq!(sybil_penalty(&a), 0.0);
    }

    #[test]
    fn tight_large_cluster_is_heavily_penalised() {
        let mut a = AgentView::sample();
        a.cluster.cluster_size = 100;
        a.cluster.suspicion = 1.0;
        assert!(sybil_penalty(&a) > 0.9);
    }

    /// Size and suspicion are both required: max suspicion in a mere pair is far
    /// gentler than the same suspicion across a large cluster.
    #[test]
    fn a_suspicious_pair_is_gentler_than_a_suspicious_swarm() {
        let mut pair = AgentView::sample();
        pair.cluster.cluster_size = 2;
        pair.cluster.suspicion = 1.0;

        let mut swarm = AgentView::sample();
        swarm.cluster.cluster_size = 200;
        swarm.cluster.suspicion = 1.0;

        assert!(sybil_penalty(&pair) < sybil_penalty(&swarm));
    }
}
