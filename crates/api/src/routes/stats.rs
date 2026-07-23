//! The aggregate statistics endpoint — the research post's headline number.
//!
//! `GET /api/stats` returns the top-line figures: how many agents exist, and —
//! the punchline — what fraction of on-chain agent reputation is manufactured.

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::error::ApiResult;
use crate::AppState;

/// The aggregate numbers. Every field is fodder for a chart in the write-up.
#[derive(Debug, Serialize)]
pub struct Stats {
    /// Total agents indexed across all chains.
    pub total_agents: i64,
    /// How many are in a flagged Sybil cluster.
    pub agents_in_clusters: i64,
    /// How many had a live endpoint at last probe.
    pub live_agents: i64,
    /// The headline: fraction of feedback edges whose target is a clustered
    /// (manufactured-looking) agent. A rough "how much reputation is fake".
    pub fake_reputation_fraction: f64,
}

/// `GET /api/stats` — a handful of aggregate queries assembled into `Stats`.
///
/// `query_scalar` is the shortcut for a query that returns a single column/value.
pub async fn summary(State(state): State<AppState>) -> ApiResult<Json<Stats>> {
    let total_agents: i64 = sqlx::query_scalar("SELECT count(*) FROM agents")
        .fetch_one(&state.db)
        .await?;

    let agents_in_clusters: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT DISTINCT chain, agent_id FROM cluster_members) t",
    )
    .fetch_one(&state.db)
    .await?;

    let live_agents: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_enrichment WHERE endpoint_healthy")
            .fetch_one(&state.db)
            .await?;

    // Fraction of all feedback that points at a clustered agent. `NULLIF(..,0)`
    // avoids a divide-by-zero when there's no feedback yet; `COALESCE(..,0.0)`
    // then turns the resulting NULL back into 0.
    let fake_reputation_fraction: f64 = sqlx::query_scalar(
        "SELECT COALESCE( \
            (SELECT count(*) FROM feedback f \
             WHERE EXISTS ( \
                SELECT 1 FROM cluster_members cm \
                WHERE cm.chain = f.chain AND cm.agent_id = f.to_agent_id \
             ))::double precision \
            / NULLIF((SELECT count(*) FROM feedback), 0), \
         0.0)",
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(Stats {
        total_agents,
        agents_in_clusters,
        live_agents,
        fake_reputation_fraction,
    }))
}
