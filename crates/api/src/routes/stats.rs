//! Aggregate statistics — raw counts for the research post. No fractions of
//! "fake reputation": the report derives its own claims from flags + evidence.

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::AppState;
use crate::error::ApiResult;

/// The aggregate numbers. Every field is fodder for a chart in the write-up.
#[derive(Debug, Serialize)]
pub struct Stats {
    pub total_agents: i64,
    pub live_endpoints: i64,
    pub payable_endpoints: i64,
    pub metadata_resolving: i64,
    pub flagged_agents: i64,
    /// Flag counts by kind, e.g. {"shared_operator": 12, ...}.
    pub flags_by_kind: serde_json::Value,
}

/// `GET /api/stats` — a handful of aggregate queries assembled into `Stats`.
///
/// `query_scalar` is the shortcut for a query that returns a single value.
pub async fn summary(State(state): State<AppState>) -> ApiResult<Json<Stats>> {
    let total_agents: i64 = sqlx::query_scalar("SELECT count(*) FROM agents")
        .fetch_one(&state.db)
        .await?;

    let live_endpoints: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_enrichment WHERE endpoint_healthy")
            .fetch_one(&state.db)
            .await?;

    // "Payable" = at least one 402 observed in the probe history — the x402
    // signal, and a count nobody else is publishing.
    let payable_endpoints: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT (chain, agent_id)) FROM probe_history WHERE outcome = 'payment_required'",
    )
    .fetch_one(&state.db)
    .await?;

    let metadata_resolving: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT (chain, agent_id)) FROM metadata_snapshots WHERE body IS NOT NULL",
    )
    .fetch_one(&state.db)
    .await?;

    let flagged_agents: i64 =
        sqlx::query_scalar("SELECT count(DISTINCT (chain, agent_id)) FROM flags")
            .fetch_one(&state.db)
            .await?;

    let flags_by_kind: serde_json::Value = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_object_agg(kind, n), '{}'::jsonb) \
         FROM (SELECT kind, count(*) AS n FROM flags GROUP BY kind) t",
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(Stats {
        total_agents,
        live_endpoints,
        payable_endpoints,
        metadata_resolving,
        flagged_agents,
        flags_by_kind,
    }))
}
