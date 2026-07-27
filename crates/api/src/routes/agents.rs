//! JSON API endpoints about agents — facts and flags, never judgments.
//!
//! Read each handler as: "the arguments say what I need from the request; the
//! return type says what I give back." That symmetry is the whole mental model
//! for axum handlers.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;

use crate::error::{ApiError, ApiResult};
use crate::facts_view::{self, AgentFacts, AgentSummary};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub chain: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    /// Explicit, objective orderings only — a "smart" default ranking would be
    /// the scalar score sneaking back in through the UI.
    ///   registered (default) — newest registration first
    ///   alive               — live endpoints first, then newest
    #[serde(default)]
    pub sort: Option<String>,
}

fn default_limit() -> i64 {
    100
}

/// `GET /api/agents` — list agents with facts-summary columns.
pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> ApiResult<Json<Vec<AgentSummary>>> {
    // Clamp: a missing limit gets a sane default, a hostile one gets a ceiling.
    let limit = params.limit.clamp(1, 500);
    let order = match params.sort.as_deref() {
        Some("alive") => "endpoint_alive DESC, registered_at DESC",
        _ => "registered_at DESC",
    };

    // `order` is interpolated, but only from the fixed match arms above —
    // user input never reaches the SQL string.
    let sql = format!(
        "SELECT a.chain, a.agent_id, a.domain, a.address_norm AS address, a.registered_at, \
                COALESCE(e.endpoint_healthy, false) AS endpoint_alive, \
                COALESCE(fl.n, 0) AS flag_count \
         FROM agents a \
         LEFT JOIN agent_enrichment e ON e.chain = a.chain AND e.agent_id = a.agent_id \
         LEFT JOIN (SELECT chain, agent_id, count(*) AS n FROM flags GROUP BY chain, agent_id) fl \
                ON fl.chain = a.chain AND fl.agent_id = a.agent_id \
         WHERE ($2::text IS NULL OR a.chain = $2) \
         ORDER BY {order} \
         LIMIT $1"
    );
    let rows = sqlx::query_as::<_, AgentSummary>(&sql)
        .bind(limit)
        .bind(&params.chain)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows))
}

/// `GET /api/agents/{chain}/{id}` — one agent's summary + facts + flags.
/// Chain is in the path: (chain, agent_id) is the identity, never id alone —
/// agent #7 on Base and agent #7 on Ethereum are different agents.
pub async fn get_one(
    State(state): State<AppState>,
    Path((chain, agent_id)): Path<(String, i64)>,
) -> ApiResult<Json<AgentFacts>> {
    let facts = facts_view::assemble(&state.db, &chain, agent_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(facts))
}

/// `GET /api/agents/{chain}/{id}/facts` — just the fact list (same assembly).
pub async fn get_facts(
    State(state): State<AppState>,
    Path((chain, agent_id)): Path<(String, i64)>,
) -> ApiResult<Json<Vec<facts::PublishedFact>>> {
    let assembled = facts_view::assemble(&state.db, &chain, agent_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(assembled.facts))
}
