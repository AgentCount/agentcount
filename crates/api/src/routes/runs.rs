//! Runs — the unit of provenance. Every fact this API serves is scoped to
//! exactly one run, and a run carries everything needed to reproduce it: the
//! pinned block, the checker version and commit, the spec commit it judged
//! against, and the literal command that re-creates it.

use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppState;
use crate::error::{ApiError, ApiResult};

/// One run, exactly as `runs` records it — no derived fields, so nothing here
/// can drift from what was actually written at sweep time.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct RunRow {
    pub run_id: Uuid,
    pub chain: String,
    pub pinned_block: Option<i64>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// `running`, `finished`, `failed` or `stalled`.
    ///
    /// Served because `finished_at` alone cannot tell a consumer whether a run
    /// is usable: a failed sweep is stamped with the moment it died, so the two
    /// terminal-but-empty states look identical to a completed one from
    /// outside. Omitting this column is what let the site default to a failed
    /// run and report 0 agents on Base — it had no way to know. See
    /// [`latest_completed`].
    pub status: String,
    /// NULL until the sweep finishes — never coalesced into 0, which would
    /// claim an in-flight run had already counted zero agents.
    pub agent_count: Option<i32>,
    pub schema_version: i32,
    pub checker_version: String,
    pub checker_commit: String,
    pub spec_commit: String,
    pub rerun_command: String,
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub chain: Option<String>,
}

/// `GET /api/runs` — every run, newest first, with full provenance.
pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> ApiResult<Json<Vec<RunRow>>> {
    let chain = params
        .chain
        .map(|c| c.trim().to_lowercase())
        .filter(|c| !c.is_empty());
    let rows = sqlx::query_as::<_, RunRow>(
        "SELECT run_id, chain, pinned_block, started_at, finished_at, status, agent_count, \
                schema_version, checker_version, checker_commit, spec_commit, rerun_command \
         FROM runs \
         WHERE ($1::text IS NULL OR chain = $1) \
         ORDER BY started_at DESC",
    )
    .bind(&chain)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

/// The latest run whose sweep has finished, optionally scoped to one chain.
/// Used to fill in `run=` when a caller omits it — an in-flight run (like the
/// one this very rewrite must not disturb) is never picked as a default,
/// because its counts are still changing under the reader's feet.
///
/// ## `finished_at IS NOT NULL` is not "this run succeeded"
///
/// It used to be the whole predicate here, and it put a **failed** run on the
/// site. `fail_run` stamps `finished_at` when a sweep dies, which is correct
/// bookkeeping — the run did end, at that moment — but it means the column
/// answers "did this stop?" and not "is there anything in it".
///
/// Run `24d4d0e0` is what that costs. It failed after two and a half minutes
/// on 2026-08-05 with zero `agent_snapshots` and zero `check_results`, and
/// because it is the most recent base run with a `finished_at`, every base
/// view defaulted to it: the directory told visitors there were
/// **0 agents on Base** while a complete 60,589-agent run sat one row down.
///
/// So the status is checked too. `finished` is the only value a sweep writes
/// on success; `failed` and `stalled` are both terminal and both empty or
/// partial.
///
/// Returns [`ApiError::NotFound`] rather than `Option` so every caller gets
/// the same 404 shape for "no completed run exists yet" without repeating
/// the `.ok_or(ApiError::NotFound)` at each call site.
pub async fn latest_completed(pool: &sqlx::PgPool, chain: Option<&str>) -> ApiResult<Uuid> {
    let run_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT run_id FROM runs \
         WHERE finished_at IS NOT NULL AND status = 'finished' \
           AND ($1::text IS NULL OR chain = $1) \
         ORDER BY started_at DESC LIMIT 1",
    )
    .bind(chain)
    .fetch_optional(pool)
    .await?;
    run_id.ok_or(ApiError::NotFound)
}
