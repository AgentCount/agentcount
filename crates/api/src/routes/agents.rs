//! JSON API endpoints about agents.
//!
//! Three handlers, each an `async fn`. Read them as: "the arguments say what I
//! need from the request; the return type says what I give back." That symmetry
//! is the whole mental model for axum handlers.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};
use crate::AppState;

/// Query-string parameters for the list endpoint, e.g.
/// `/api/agents?chain=base&limit=50&sort=score`.
///
/// axum's `Query` extractor uses serde to parse these off the URL into this
/// struct. `Option`/`#[serde(default)]` fields make each parameter optional.
#[derive(Debug, Deserialize)]
pub struct ListParams {
    /// Filter by chain ("ethereum" / "base"). `None` = all chains.
    pub chain: Option<String>,
    /// Max rows to return. Defaulted so a missing `limit` doesn't mean "no cap".
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    100
}

/// One agent as returned by the list endpoint. `#[derive(Serialize)]` is what
/// lets `Json(..)` turn a `Vec<AgentSummary>` into a JSON array.
#[derive(Debug, Serialize)]
pub struct AgentSummary {
    pub agent_id: u64,
    pub chain: String,
    pub domain: String,
    pub final_score: f64,
}

/// `GET /api/agents` — list agents, filtered and sorted.
///
/// Reading the signature: we take shared `State` (to reach the db) and the parsed
/// `Query` params; we return `ApiResult<Json<...>>` so we can `?` on failures and
/// hand back JSON on success. axum wraps the `Vec` in a JSON response with the
/// right content-type automatically.
pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> ApiResult<Json<Vec<AgentSummary>>> {
    // Sketch:
    //   * Build a query that optionally filters by `params.chain` and orders by
    //     the latest final_score, LIMIT `params.limit`.
    //   * Map rows into `AgentSummary`.
    //   * `Ok(Json(rows))`
    //
    // The `?` on a query would convert a `sqlx::Error` into an `ApiError` via the
    // `From` impl sketched in error.rs — that's the payoff of centralising errors.
    let _ = (state, params);
    todo!("query agents (filtered/sorted/limited) and return them as JSON")
}

/// `GET /api/agents/{id}` — one agent with its enrichment.
///
/// The `Path(agent_id)` extractor pulls `{id}` out of the URL and parses it into
/// a `u64`. If it isn't a valid number, axum rejects the request before your
/// handler even runs — invalid input can't reach your logic.
pub async fn get_one(
    State(state): State<AppState>,
    Path(agent_id): Path<u64>,
) -> ApiResult<Json<AgentSummary>> {
    // Fetch the agent; if the query finds no row, return `ApiError::NotFound`
    // (the `From<sqlx::Error>` impl maps `RowNotFound` → 404 for you).
    let _ = (state, agent_id);
    todo!("fetch one agent by id or return ApiError::NotFound")
}

/// `GET /api/agents/{id}/score` — the full trust-score breakdown.
///
/// This is the handler that actually calls the `scoring` library: it assembles
/// an `AgentView` from the database, then hands it to `scoring::score`. Note how
/// the pure library and the I/O live cleanly on opposite sides of this call.
pub async fn get_score(
    State(state): State<AppState>,
    Path(agent_id): Path<u64>,
) -> ApiResult<Json<scoring::TrustScore>> {
    // Sketch:
    //   1. Load everything the scorer needs from Postgres and pack it into a
    //      `scoring::AgentView` (payments, probes, feedback edges, cluster info).
    //   2. `let score = scoring::score(&view)?;`  // pure call, no I/O
    //   3. `Ok(Json(score))`
    //
    // If assembling the view finds no such agent → `ApiError::NotFound`.
    let _ = (state, agent_id);
    todo!("assemble a scoring::AgentView from the db, call scoring::score, return JSON")
}
