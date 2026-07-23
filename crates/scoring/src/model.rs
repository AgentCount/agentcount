//! Data types the scoring math operates on.
//!
//! This file defines the *shape* of the input ([`AgentView`]) and the output
//! ([`TrustScore`]), plus the tunable [`ScoreWeights`]. Keeping the types in one
//! place means the math modules can all agree on one vocabulary.
//!
//! Rust concept spotlight: **structs and derive macros.** A `struct` is a named
//! bundle of fields (like a record/POJO). `#[derive(...)]` asks the compiler to
//! auto-generate common trait implementations for us — `Debug` for `{:?}`
//! printing, `Clone` to make copies, `Serialize` so `api` can emit JSON. You'll
//! type `#[derive(...)]` hundreds of times in Rust; this is where it clicks.

use serde::Serialize;

/// Everything the scoring functions are allowed to look at about one agent.
///
/// This is an *assembled view*: some other layer (the `api` crate) gathers rows
/// from several database tables and packs them into this one struct before
/// calling [`crate::score`]. The scoring crate never runs a query itself — it
/// just receives this tidy value. That boundary is the whole point.
///
/// The fields below are a starting vocabulary. Add to them as your methodology
/// grows; the compiler will point at every place that needs updating.
#[derive(Debug, Clone)]
pub struct AgentView {
    /// The ERC-8004 agent id (unique per registry).
    pub agent_id: u64,

    /// Which chain the agent is registered on, e.g. "ethereum" or "base".
    /// (A plain `String` for now; promoting this to an `enum Chain { Ethereum,
    /// Base }` later is a great exercise in making illegal states unrepresentable.)
    pub chain: String,

    // ── Inputs for the PAYMENT sub-score ────────────────────────────────────
    /// Distinct counterparties this agent has transacted with. Diversity here is
    /// the anti-gaming signal: real demand comes from many wallets, manufactured
    /// volume comes from a handful of colluding ones.
    pub distinct_counterparties: u64,
    /// Total economic value moved (in some normalized unit — decide and document
    /// it, e.g. USD-at-time-of-tx). Log-scaled in the math so whales don't
    /// dominate linearly.
    pub total_payment_value: f64,

    // ── Inputs for the LIVENESS sub-score ───────────────────────────────────
    /// How many times the enricher probed the endpoint.
    pub probe_count: u32,
    /// How many of those probes succeeded AND returned a valid agent-card.
    pub probe_successes: u32,

    // ── Inputs for the AGE sub-score ────────────────────────────────────────
    /// When the agent first registered on-chain.
    pub first_seen: chrono::DateTime<chrono::Utc>,
    /// The most recent activity we observed. A long, well-spread history beats a
    /// burst of activity crammed into one afternoon.
    pub last_activity: chrono::DateTime<chrono::Utc>,
    /// Number of distinct days on which the agent was active. Spread, not just span.
    pub active_days: u32,

    // ── Inputs for the REPUTATION sub-score ─────────────────────────────────
    /// Feedback attestations pointing *at* this agent, already annotated with how
    /// independent/trusted each attester is (see [`FeedbackEdge`]).
    pub incoming_feedback: Vec<FeedbackEdge>,

    // ── Inputs for the SYBIL penalty ────────────────────────────────────────
    /// What the enricher's clustering stage concluded about this agent.
    pub cluster: ClusterInfo,
}

/// One piece of feedback pointing at the agent being scored.
///
/// The key idea: not all praise is equal. Feedback from a well-established,
/// independent agent should count for more than feedback from a brand-new
/// account that only ever rates its friends. We carry those signals here so the
/// reputation math can weight each edge.
#[derive(Debug, Clone)]
pub struct FeedbackEdge {
    /// The agent giving the feedback.
    pub from_agent_id: u64,
    /// The raw score they attested, already normalised to `[0, 1]`.
    pub raw_value: f64,
    /// A pre-computed trust weight for the *attester* — how much this source's
    /// opinion should count. In a full EigenTrust-style system this would be the
    /// attester's own (recursively computed) trust. Start with a simpler proxy.
    pub attester_weight: f64,
    /// Did the scored agent also give feedback back to `from_agent_id`? Mutual
    /// A↔B ratings are a classic collusion pattern and get discounted.
    pub is_reciprocal: bool,
}

/// What the clustering stage found out about an agent's "neighbourhood".
///
/// A "cluster" is a group of agents that look suspiciously coordinated — shared
/// funding wallet, shared operator address, near-simultaneous registration,
/// dense mutual-rating rings. The bigger and tighter the cluster, the more its
/// members' reputations are probably manufactured.
#[derive(Debug, Clone)]
pub struct ClusterInfo {
    /// The cluster this agent belongs to, if any. `Option<T>` is Rust's built-in
    /// "maybe a value, maybe nothing" — there is no `null`; you must handle the
    /// `None` case, which the compiler enforces.
    pub cluster_id: Option<uuid::Uuid>,
    /// How many agents are in that cluster (1 = effectively alone).
    pub cluster_size: u32,
    /// A `[0, 1]` "how coordinated does this cluster look" signal produced by the
    /// enricher. 0 = looks organic, 1 = looks like a bot farm.
    pub suspicion: f64,
}

/// The result of scoring: the four positive sub-scores, the penalty, and the
/// combined final number. Every field is in `[0, 1]`.
///
/// `#[derive(Serialize)]` is what lets the `api` crate turn this straight into
/// the JSON your `/api/agents/:id/score` endpoint returns — and, conveniently,
/// what makes the score breakdown easy to show on the explorer page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrustScore {
    pub payment: f64,
    pub liveness: f64,
    pub age: f64,
    pub reputation: f64,
    /// The multiplicative penalty (0 = no penalty, 1 = score annihilated).
    pub sybil_penalty: f64,
    /// `raw · (1 − sybil_penalty)` — the single number the leaderboard sorts by.
    pub final_score: f64,
}

/// The tunable weights that decide how much each positive sub-score matters.
///
/// These are the knobs of the *published methodology*. Keeping them in one
/// struct (rather than as magic numbers sprinkled through the code) means the
/// whole methodology is one auditable, serializable value.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScoreWeights {
    pub payment: f64,
    pub liveness: f64,
    pub age: f64,
    pub reputation: f64,
}

impl ScoreWeights {
    /// Check that the weights form a valid convex combination (sum ≈ 1.0). If
    /// they don't, `raw` could exceed 1.0 and the final score would leave its
    /// intended range.
    ///
    /// `&self` means "borrow the weights to read them"; we return the crate's
    /// error type so callers can react to a bad configuration.
    ///
    /// We compare with a tiny tolerance rather than `==` because floating-point
    /// sums are rarely *exactly* 1.0 (adding `0.40, 0.20, 0.15, 0.25` may land a
    /// hair off). Comparing floats for exact equality is a classic bug; always
    /// compare within a tolerance.
    pub fn validate(&self) -> Result<(), crate::ScoringError> {
        let sum = self.payment + self.liveness + self.age + self.reputation;
        if (sum - 1.0).abs() > 1e-9 {
            return Err(crate::ScoringError::WeightsDoNotSumToOne { actual: sum });
        }
        Ok(())
    }
}

/// `Default` is a standard trait for "the sensible out-of-the-box value". Once
/// implemented, `ScoreWeights::default()` gives the published baseline weights.
/// Deriving vs hand-writing: we hand-write it here so we can choose meaningful
/// numbers (a derive would just zero everything, which wouldn't sum to 1.0).
impl Default for ScoreWeights {
    fn default() -> Self {
        // A defensible starting split — tune these as the research demands. They
        // MUST sum to 1.0 (see `validate`). Reputation is weighted the lowest on
        // purpose: it is the easiest signal to fake, so we lean on harder-to-fake
        // economic and liveness evidence instead.
        Self {
            payment: 0.40,
            liveness: 0.20,
            age: 0.15,
            reputation: 0.25,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test-only helper: a believable baseline agent that tests can tweak.
// ─────────────────────────────────────────────────────────────────────────────
//
// `#[cfg(test)]` means this block is compiled ONLY during `cargo test`. Because
// it's part of the same crate, the sub-score test modules (in subscores/*.rs)
// can call `AgentView::sample()` too — `cfg(test)` is active for the whole crate
// when testing. Every test starts from this healthy baseline and mutates just
// the one field it cares about, so each test states exactly one idea.
#[cfg(test)]
impl AgentView {
    /// A healthy, organic-looking agent. Clone it and change one field per test.
    pub(crate) fn sample() -> Self {
        // Fixed timestamps (no `Utc::now()`): scoring must be deterministic, and
        // so must its tests. `from_timestamp` builds a UTC time from a Unix epoch
        // second count; `.unwrap()` is fine here because the constant is valid.
        let first_seen = chrono::DateTime::from_timestamp(1_600_000_000, 0).unwrap();
        let last_activity = first_seen + chrono::Duration::days(180);

        Self {
            agent_id: 1,
            chain: "ethereum".to_string(),
            distinct_counterparties: 40,
            total_payment_value: 5_000.0,
            probe_count: 10,
            probe_successes: 9,
            first_seen,
            last_activity,
            active_days: 90,
            incoming_feedback: vec![FeedbackEdge {
                from_agent_id: 2,
                raw_value: 1.0,
                attester_weight: 1.0,
                is_reciprocal: false,
            }],
            cluster: ClusterInfo {
                cluster_id: None,
                cluster_size: 1,
                suspicion: 0.0,
            },
        }
    }
}
