//! Sybil detection — the hardest and most interesting part of the enricher.
//!
//! A Sybil attack is one operator controlling many agents that *appear*
//! independent. Detecting it is fundamentally a **graph problem**: build a graph
//! where agents are nodes and edges connect agents that share tell-tale signals,
//! then find the tightly-connected clusters. Members of a big, tight cluster are
//! probably one puppeteer, so the scorer penalises them (via the sybil_penalty).
//!
//! ## The signals we link on (edges)
//!
//! No single signal is proof; the strength comes from combining them:
//!   * **Shared funding source** — agents first funded by the same wallet.
//!   * **Shared operator address** — the same address controls several agents.
//!   * **Synchronised registration** — a burst of agents registered within
//!     minutes of each other (a script running down a list).
//!   * **Reciprocal-feedback rings** — dense mutual rating that no organic set of
//!     strangers would produce.
//!
//! ## Rust concepts this crate is here to teach
//!
//! * **Borrowing a big data structure to compute over it** — `detect` takes
//!   `&Db` and reads a lot; the graph it builds is owned locally and dropped
//!   when done. You'll feel ownership working *for* you: once the function
//!   returns its results, all the scratch memory is freed automatically, no GC.
//! * **`HashMap`/`HashSet` and iterators** — the bread-and-butter of graph code
//!   in Rust. Building adjacency lists is a great `HashMap<K, Vec<V>>` exercise.

use anyhow::Result;
use crate::store::Db;

/// One detected cluster of suspiciously-coordinated agents, ready to persist.
pub struct Cluster {
    /// The agents in this cluster.
    pub agent_ids: Vec<u64>,
    /// Which signal(s) bound them together — useful to show on the explorer so
    /// the methodology is transparent, not a black box.
    pub reasons: Vec<ClusterReason>,
    /// A `[0, 1]` "how coordinated does this look" score. This becomes the
    /// `suspicion` field the scorer reads via `ClusterInfo`. Bigger + tighter +
    /// more-independent-signals ⇒ closer to 1.0.
    pub suspicion: f64,
}

/// Why a cluster was flagged. An enum keeps the reasons typed and displayable,
/// and forces us to name each detection heuristic explicitly.
#[derive(Debug, Clone)]
pub enum ClusterReason {
    SharedFundingSource,
    SharedOperatorAddress,
    SynchronisedRegistration,
    ReciprocalFeedbackRing,
}

/// Run clustering across ALL agents and return the detected clusters.
///
/// This needs the global picture (you can't spot a ring by looking at one agent
/// in isolation), which is why it takes the whole `Db` and runs as its own pass
/// rather than per-agent inside the probe loop.
pub async fn detect(db: &Db) -> Result<Vec<Cluster>> {
    // High-level plan (each step is its own satisfying sub-problem):
    //
    // 1. LOAD the raw relationships from Postgres:
    //      * funding edges  (agent → first funder address)
    //      * operator edges (agent → controlling address)
    //      * registration timestamps per agent
    //      * the feedback graph (from_agent → to_agent)
    //
    // 2. BUILD an undirected graph. A `HashMap<u64, HashSet<u64>>` adjacency list
    //    is the natural representation: for each pair of agents that share a
    //    signal, add an edge. For "synchronised registration", sort agents by
    //    timestamp and link ones registered within a small time window.
    //
    //         use std::collections::{HashMap, HashSet};
    //         let mut adj: HashMap<u64, HashSet<u64>> = HashMap::new();
    //         // ... for each shared signal: adj.entry(a).or_default().insert(b);
    //
    // 3. FIND connected components (each component = one candidate cluster). A
    //    breadth-first or union-find pass over the adjacency map. Singletons
    //    (components of size 1) aren't clusters — drop them.
    //
    // 4. SCORE each component's `suspicion` from its size, its internal edge
    //    density, and how many *distinct* signal types bind it (a group linked by
    //    shared funding AND a feedback ring is far more damning than one linked
    //    by a single weak signal).
    //
    // 5. RETURN the clusters; `main` persists them and the scorer reads the
    //    resulting per-agent suspicion.

    let _ = db;
    todo!("build the agent-relationship graph, find components, score suspicion")
}
