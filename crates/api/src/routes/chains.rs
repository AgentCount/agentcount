//! The chain list — what a frontend's chain filter is allowed to offer.
//!
//! Sourced from the `chains` table (migration 0004), filtered to enabled rows,
//! so the filter can never offer a chain the indexer is not running.

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::AppState;
use crate::error::ApiResult;

/// One indexed chain, with how many agents we have on it.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ChainRow {
    pub chain: String,
    pub chain_id: i64,
    /// Agents indexed on this chain — enough for a filter to show counts
    /// without a second round trip.
    pub agents: i64,
}

/// `GET /api/chains` — enabled chains, alphabetical.
pub async fn list(State(state): State<AppState>) -> ApiResult<Json<Vec<ChainRow>>> {
    // LEFT JOIN so a freshly-seeded chain with no agents yet still appears
    // (an INNER JOIN would silently hide it).
    let rows = sqlx::query_as::<_, ChainRow>(
        "SELECT c.chain, c.chain_id, count(a.agent_id) AS agents \
         FROM chains c \
         LEFT JOIN agents a ON a.chain = c.chain \
         WHERE c.enabled \
         GROUP BY c.chain, c.chain_id \
         ORDER BY c.chain",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}
