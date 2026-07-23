//! The aggregate statistics endpoint — your launch-post headline number lives here.
//!
//! `GET /api/stats` returns the top-line figures that make the research post:
//! how many agents exist, and — the punchline — what fraction of on-chain agent
//! reputation is manufactured. This is the single endpoint the blog post quotes.

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::error::ApiResult;
use crate::AppState;

/// The aggregate numbers. Every field is fodder for a chart in the write-up.
#[derive(Debug, Serialize)]
pub struct Stats {
    /// Total agents indexed across all chains.
    pub total_agents: u64,
    /// How many are in a flagged Sybil cluster.
    pub agents_in_clusters: u64,
    /// The headline: fraction of total *reputation* (feedback weight) that
    /// belongs to clustered/penalised agents. This is "how much is fake".
    pub fake_reputation_fraction: f64,
    /// How many agents had a live endpoint at last probe — a reality check on
    /// how much of the registered population actually functions.
    pub live_agents: u64,
}

/// `GET /api/stats` — compute and return the aggregates.
///
/// A handler that needs only the database: one `State` argument in, `Json` out.
pub async fn summary(State(state): State<AppState>) -> ApiResult<Json<Stats>> {
    // Sketch: a handful of aggregate SQL queries (COUNT, SUM over feedback weight
    // partitioned by whether the target agent is clustered), assembled into a
    // `Stats`. Consider caching this — it's read constantly by the landing page
    // and changes slowly.
    let _ = state;
    todo!("run the aggregate queries and assemble a Stats value")
}
