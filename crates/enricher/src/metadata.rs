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
    /// The domain the agent registered on-chain. Attacker-controlled: only the
    /// netguard-checked observe() path may turn this into a request.
    pub domain: String,
}
