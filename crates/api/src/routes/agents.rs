//! JSON API endpoints about agents — facts and flags, never judgments.
//!
//! Read each handler as: "the arguments say what I need from the request; the
//! return type says what I give back." That symmetry is the whole mental model
//! for axum handlers.

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::AppState;
use crate::error::{ApiError, ApiResult};
use crate::facts_view::{self, AgentFacts, AgentSummary};

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub chain: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
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

/// `GET /api/agents` — one page of the directory, plus where that page sits.
pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> ApiResult<Json<facts_view::Page<AgentSummary>>> {
    // Clamp: a missing limit gets a sane default, a hostile one gets a ceiling.
    // A negative offset is meaningless, not an error — treat it as the start.
    let filter = facts_view::ListFilter {
        // An empty (or whitespace-only) `chain` means "no filter, all
        // chains" — not a chain literally named "" that matches nothing.
        // `chains.chain`/`agents.chain` are canonical lowercase, so normalize
        // case here too rather than surprising a caller who sends `?chain=BASE`.
        chain: params
            .chain
            .map(|c| c.trim().to_lowercase())
            .filter(|c| !c.is_empty()),
        limit: params.limit.clamp(1, 500),
        offset: params.offset.max(0),
        sort: facts_view::Sort::from_param(params.sort.as_deref()),
    };
    Ok(Json(facts_view::list_agents(&state.db, &filter).await?))
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
