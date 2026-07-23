//! Sybil detection — the hardest and most interesting part of the enricher.
//!
//! A Sybil attack is one operator controlling many agents that *appear*
//! independent. Detecting it is fundamentally a **graph problem**: build a graph
//! where agents are nodes and edges connect agents that share a tell-tale
//! signal, then find the tightly-connected clusters. Members of a big, tight
//! cluster are probably one puppeteer, so the scorer penalises them.
//!
//! ## The signals we link on (edges)
//!
//! No single signal is proof; the strength comes from combining them:
//!   * **Shared operator address** — the same wallet controls several agents.
//!   * **Synchronised registration** — a burst of agents registered within a
//!     short window of each other (a script running down a list).
//!   * **Reciprocal-feedback rings** — A rates B *and* B rates A, the mutual
//!     back-scratch no set of strangers would produce.
//!
//! (A fourth signal, *shared funding source*, is in [`ClusterReason`] but needs a
//! data source we don't index yet — see the note there.)
//!
//! ## Rust concepts this module is here to teach
//!
//! * **Union-Find (disjoint-set)** — a tiny, classic data structure for grouping
//!   things into connected components. Implemented from scratch below so you can
//!   see exactly how it works.
//! * **`HashMap`/`HashSet` and iterators** — the bread and butter of graph code.

use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::store::Db;

/// Agents are identified by (chain, id): the same numeric id can exist on both
/// Ethereum and Base, so the chain is part of the identity. Deriving `Hash`,
/// `Eq`, and `Clone` lets us use this as a `HashMap` key and copy it freely.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentKey {
    pub chain: String,
    pub agent_id: i64,
}

/// The per-agent facts clustering reasons over, loaded from the database.
#[derive(Debug, Clone)]
pub struct AgentNode {
    pub key: AgentKey,
    /// The controlling wallet address (lower-cased hex).
    pub address: String,
    pub registered_at: chrono::DateTime<chrono::Utc>,
}

/// Why a cluster was flagged. An enum keeps the reasons typed and displayable,
/// and forces us to name each detection heuristic explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClusterReason {
    /// Not yet detected: we don't index first-funder data. Kept so the intent is
    /// documented and adding it later is a one-line change here. `#[allow(...)]`
    /// silences the "never constructed" warning until you wire up funding data.
    #[allow(dead_code)]
    SharedFundingSource,
    SharedOperatorAddress,
    SynchronisedRegistration,
    ReciprocalFeedbackRing,
}

impl ClusterReason {
    /// Stable string stored in the `clusters.reasons` JSONB array and shown on
    /// the explorer, so the methodology is transparent.
    pub fn label(&self) -> &'static str {
        match self {
            ClusterReason::SharedFundingSource => "shared_funding_source",
            ClusterReason::SharedOperatorAddress => "shared_operator_address",
            ClusterReason::SynchronisedRegistration => "synchronised_registration",
            ClusterReason::ReciprocalFeedbackRing => "reciprocal_feedback_ring",
        }
    }
}

/// One detected cluster of suspiciously-coordinated agents, ready to persist.
pub struct Cluster {
    pub members: Vec<AgentKey>,
    /// Which signal(s) bound them together.
    pub reasons: Vec<ClusterReason>,
    /// A `[0, 1]` "how coordinated does this look" score. This becomes each
    /// member's `suspicion`, which the scorer reads. It reflects internal edge
    /// density and how many *distinct* signals bind the group — NOT size, because
    /// the scorer's `sybil_penalty` already scales by cluster size (avoiding
    /// double-counting).
    pub suspicion: f64,
}

/// Agents registered within this many seconds of each other are linked as a
/// "synchronised registration" burst.
const REGISTRATION_WINDOW_SECS: i64 = 120;

/// Run clustering across ALL agents and return the detected clusters.
///
/// Needs the global picture (you can't spot a ring by looking at one agent in
/// isolation), which is why it takes the whole `Db` and runs as its own pass.
pub async fn detect(db: &Db) -> Result<Vec<Cluster>> {
    let nodes = db.load_agent_nodes().await?;
    let feedback = db.load_feedback_pairs().await?; // directed (from, to) edges

    if nodes.is_empty() {
        return Ok(vec![]);
    }

    // Map each agent to a dense index 0..n so union-find can use a plain Vec.
    let index: HashMap<AgentKey, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.key.clone(), i))
        .collect();

    let mut uf = UnionFind::new(nodes.len());

    // Collect the undirected edges we add, each tagged with WHY, so we can later
    // measure a component's density and distinct-reason count. Keyed by an
    // ordered pair (min, max) so we don't count an edge twice.
    let mut edge_reasons: HashMap<(usize, usize), HashSet<ClusterReason>> = HashMap::new();
    let mut add_edge = |uf: &mut UnionFind, a: usize, b: usize, reason: ClusterReason| {
        if a == b {
            return;
        }
        let pair = if a < b { (a, b) } else { (b, a) };
        edge_reasons.entry(pair).or_default().insert(reason);
        uf.union(a, b);
    };

    // ── Signal 1: shared operator address ───────────────────────────────────
    // Group agent indices by address; every agent in a group of size > 1 is
    // linked to the others (we link each to the group's first member — enough to
    // make them one component).
    let mut by_address: HashMap<&str, Vec<usize>> = HashMap::new();
    for node in &nodes {
        by_address
            .entry(node.address.as_str())
            .or_default()
            .push(index[&node.key]);
    }
    for group in by_address.values() {
        for &other in &group[1..] {
            add_edge(&mut uf, group[0], other, ClusterReason::SharedOperatorAddress);
        }
    }

    // ── Signal 2: synchronised registration ─────────────────────────────────
    // Sort by registration time; link consecutive agents that registered within
    // the window. A tight burst becomes one connected chain.
    let mut by_time: Vec<&AgentNode> = nodes.iter().collect();
    by_time.sort_by_key(|n| n.registered_at);
    for pair in by_time.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let gap = (b.registered_at - a.registered_at).num_seconds();
        if gap <= REGISTRATION_WINDOW_SECS {
            add_edge(
                &mut uf,
                index[&a.key],
                index[&b.key],
                ClusterReason::SynchronisedRegistration,
            );
        }
    }

    // ── Signal 3: reciprocal feedback rings ─────────────────────────────────
    // Build a set of directed edges, then any (a→b) that also has (b→a) is a
    // mutual pair worth linking.
    let directed: HashSet<(AgentKey, AgentKey)> = feedback.iter().cloned().collect();
    for (from, to) in &feedback {
        if directed.contains(&(to.clone(), from.clone())) {
            // Both endpoints must be agents we know about.
            if let (Some(&a), Some(&b)) = (index.get(from), index.get(to)) {
                add_edge(&mut uf, a, b, ClusterReason::ReciprocalFeedbackRing);
            }
        }
    }

    // ── Assemble components into clusters ────────────────────────────────────
    // Group indices by their union-find root.
    let mut components: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..nodes.len() {
        components.entry(uf.find(i)).or_default().push(i);
    }

    let mut clusters = Vec::new();
    for members in components.values() {
        // Singletons aren't clusters.
        if members.len() < 2 {
            continue;
        }
        let member_set: HashSet<usize> = members.iter().copied().collect();

        // Gather the edges (and reasons) that fall INSIDE this component.
        let mut reasons = HashSet::new();
        let mut internal_edges = 0usize;
        for (&(a, b), rs) in &edge_reasons {
            if member_set.contains(&a) && member_set.contains(&b) {
                internal_edges += 1;
                reasons.extend(rs.iter().copied());
            }
        }

        let suspicion = compute_suspicion(members.len(), internal_edges, reasons.len());

        clusters.push(Cluster {
            members: members.iter().map(|&i| nodes[i].key.clone()).collect(),
            reasons: reasons.into_iter().collect(),
            suspicion,
        });
    }

    Ok(clusters)
}

/// Turn a component's shape into a `[0, 1]` coordination score.
///
/// Two ingredients, blended:
///   * **edge density** — actual edges ÷ maximum possible edges. A fully-meshed
///     group (everyone links to everyone) is far more damning than a loose chain.
///   * **reason diversity** — how many of the (currently 3 detectable) distinct
///     signals bind the group. Being linked by shared-operator AND a feedback
///     ring is stronger evidence than a single weak signal.
fn compute_suspicion(size: usize, internal_edges: usize, distinct_reasons: usize) -> f64 {
    let max_edges = (size * (size - 1)) / 2; // n choose 2
    let density = if max_edges == 0 {
        0.0
    } else {
        internal_edges as f64 / max_edges as f64
    };
    // 3 = the number of signals we can currently detect.
    let reason_strength = (distinct_reasons as f64 / 3.0).min(1.0);
    (0.6 * density + 0.4 * reason_strength).clamp(0.0, 1.0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Union-Find (a.k.a. disjoint-set union) — group items into connected components.
// ─────────────────────────────────────────────────────────────────────────────
//
// The idea: every item starts in its own group. `union(a, b)` merges two groups;
// `find(x)` returns a canonical "representative" for x's group, so two items are
// in the same group iff they share a representative. With path compression it's
// effectively O(1) per operation. This is the standard tool for "which things are
// transitively connected?".
struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        // Each element is initially its own parent (its own group).
        Self {
            parent: (0..n).collect(),
        }
    }

    /// Find the representative of `x`'s set, compressing the path as we go so
    /// future lookups are faster. `&mut self` because compression mutates state.
    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        // Path compression: point everyone on the path straight at the root.
        let mut cur = x;
        while self.parent[cur] != root {
            let next = self.parent[cur];
            self.parent[cur] = root;
            cur = next;
        }
        root
    }

    /// Merge the sets containing `a` and `b`.
    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}
