//! Base rates — the product's headline output.
//!
//! Per rung, how many agents landed in each status, for ONE run. This is the
//! one aggregate this API ever publishes: a population count, never a
//! per-agent one. `GROUP BY rung, status` is the entire query — there is no
//! second step that folds the four statuses into a single number per agent.

use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiResult;

#[derive(Debug, Serialize, sqlx::FromRow)]
struct RungStatusCount {
    rung: i16,
    status: String,
    count: i64,
}

/// One status's count within one rung.
#[derive(Debug, Serialize)]
pub struct StatusCount {
    pub status: String,
    pub count: i64,
}

/// One rung's full status breakdown.
#[derive(Debug, Serialize)]
pub struct RungRates {
    pub rung: i16,
    pub counts: Vec<StatusCount>,
}

#[derive(Debug, Serialize)]
pub struct RatesResponse {
    pub run_id: Uuid,
    /// The denominator every rung's counts are drawn from: how many agents
    /// this run snapshotted, full stop. NOT itself a per-agent pass count —
    /// it's the same number for every rung, the population size, not a score.
    pub agent_count: i64,
    pub rungs: Vec<RungRates>,
}

/// `GET /api/runs/{id}/rates` — `idx_check_results_rates` (run_id, rung,
/// status) exists for exactly this query.
pub async fn get(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> ApiResult<Json<RatesResponse>> {
    // 404 rather than an empty rungs list for an unknown run id — an id that
    // was never opened is a different claim than "opened, zero results so far".
    let agent_count: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_snapshots WHERE run_id = $1")
        .bind(run_id)
        .fetch_one(&state.db)
        .await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs WHERE run_id = $1)")
        .bind(run_id)
        .fetch_one(&state.db)
        .await?;
    if !exists {
        return Err(crate::error::ApiError::NotFound);
    }

    let raw = sqlx::query_as::<_, RungStatusCount>(
        "SELECT rung, status, count(*) AS count FROM check_results \
         WHERE run_id = $1 GROUP BY rung, status ORDER BY rung, status",
    )
    .bind(run_id)
    .fetch_all(&state.db)
    .await?;

    // Group the flat (rung, status, count) rows into one entry per rung —
    // structuring, not aggregating: every count from the query survives
    // untouched, just nested under its rung.
    let mut rungs: Vec<RungRates> = Vec::new();
    for row in raw {
        match rungs.last_mut() {
            Some(r) if r.rung == row.rung => r.counts.push(StatusCount {
                status: row.status,
                count: row.count,
            }),
            _ => rungs.push(RungRates {
                rung: row.rung,
                counts: vec![StatusCount {
                    status: row.status,
                    count: row.count,
                }],
            }),
        }
    }

    Ok(Json(RatesResponse {
        run_id,
        agent_count,
        rungs,
    }))
}
