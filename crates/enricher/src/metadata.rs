//! The lightweight view of an agent the enricher works from — just enough to
//! know where to observe it. Built from a database row in `store.rs`.
//!
//! Rust concept spotlight: **`#[derive(sqlx::FromRow)]`** maps a `SELECT chain,
//! agent_id, domain` row straight into this struct by matching column names to
//! field names. `Clone` lets the concurrent observe stage move a copy into
//! each async job.

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentStub {
    pub chain: String,
    /// Postgres BIGINT is signed, so i64 — the DB's type is the source of truth.
    pub agent_id: i64,
    /// The agent's on-chain `agentURI` (stored in the `agents.domain` column;
    /// the query aliases it). May be an https URL, a `data:` card, an `ipfs://`
    /// reference, or malformed. Attacker-controlled: only the netguard-checked
    /// `observe()` path may turn it into a request.
    pub agent_uri: String,
}
