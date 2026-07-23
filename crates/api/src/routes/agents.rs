//! JSON API endpoints about agents.
//!
//! Three handlers, each an `async fn`. Read them as: "the arguments say what I
//! need from the request; the return type says what I give back." That symmetry
//! is the whole mental model for axum handlers. All three just read Postgres —
//! the enricher already did the scoring.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};
use crate::AppState;

/// Query-string parameters for the list endpoint, e.g.
/// `/api/agents?chain=base&limit=50`. axum's `Query` extractor uses serde to
/// parse these off the URL. Optional/defaulted fields make each one optional.
#[derive(Debug, Deserialize)]
pub struct ListParams {
    /// Filter by chain ("ethereum" / "base"). `None` = all chains.
    pub chain: Option<String>,
    /// Max rows to return. Defaulted so a missing `limit` doesn't mean "no cap".
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    100
}

/// One agent as returned by the list/get endpoints. `FromRow` maps a DB row into
/// it; `Serialize` turns it into JSON. The column names in the SQL are aliased to
/// match these field names.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AgentSummary {
    pub chain: String,
    pub agent_id: i64,
    pub domain: String,
    pub final_score: f64,
}

/// `GET /api/agents` — list agents, newest-score-first, filtered and limited.
///
/// The subquery uses Postgres's `DISTINCT ON` to pick the most recent score row
/// per agent, then the outer query sorts those by score and applies the limit.
pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> ApiResult<Json<Vec<AgentSummary>>> {
    let rows = sqlx::query_as::<_, AgentSummary>(
        "SELECT latest.chain, latest.agent_id, latest.domain, latest.final_score \
         FROM ( \
            SELECT DISTINCT ON (s.chain, s.agent_id) \
                s.chain, s.agent_id, a.domain, s.final_score, s.computed_at \
            FROM scores s \
            JOIN agents a ON a.chain = s.chain AND a.agent_id = s.agent_id \
            ORDER BY s.chain, s.agent_id, s.computed_at DESC \
         ) latest \
         WHERE ($2::text IS NULL OR latest.chain = $2) \
         ORDER BY latest.final_score DESC \
         LIMIT $1",
    )
    .bind(params.limit)
    .bind(&params.chain)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows))
}

/// `GET /api/agents/{id}` — one agent with its latest final score.
///
/// `Path(agent_id)` pulls `{id}` out of the URL and parses it into an `i64`; if
/// it isn't a valid number, axum rejects the request before this runs.
/// `fetch_optional` returns `Option`, and `ok_or` turns `None` into a 404.
pub async fn get_one(
    State(state): State<AppState>,
    Path(agent_id): Path<i64>,
) -> ApiResult<Json<AgentSummary>> {
    let row = sqlx::query_as::<_, AgentSummary>(
        "SELECT a.chain, a.agent_id, a.domain, \
            COALESCE(( \
                SELECT s.final_score FROM scores s \
                WHERE s.chain = a.chain AND s.agent_id = a.agent_id \
                ORDER BY s.computed_at DESC LIMIT 1 \
            ), 0.0) AS final_score \
         FROM agents a WHERE a.agent_id = $1 LIMIT 1",
    )
    .bind(agent_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(Json(row))
}

/// `GET /api/agents/{id}/score` — the full trust-score breakdown.
///
/// We serve the latest *stored* score (the enricher computes and persists these
/// on a schedule). We reconstruct the `scoring::TrustScore` type and return it,
/// so the API and the scoring library agree on the shape by construction.
pub async fn get_score(
    State(state): State<AppState>,
    Path(agent_id): Path<i64>,
) -> ApiResult<Json<scoring::TrustScore>> {
    // A local row struct to receive the columns, then map into the library type.
    #[derive(sqlx::FromRow)]
    struct ScoreRow {
        payment: f64,
        liveness: f64,
        age: f64,
        reputation: f64,
        sybil_penalty: f64,
        final_score: f64,
    }

    let row = sqlx::query_as::<_, ScoreRow>(
        "SELECT payment, liveness, age, reputation, sybil_penalty, final_score \
         FROM scores WHERE agent_id = $1 ORDER BY computed_at DESC LIMIT 1",
    )
    .bind(agent_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(Json(scoring::TrustScore {
        payment: row.payment,
        liveness: row.liveness,
        age: row.age,
        reputation: row.reputation,
        sybil_penalty: row.sybil_penalty,
        final_score: row.final_score,
    }))
}
