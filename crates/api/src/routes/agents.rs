//! The agent directory and single-agent detail — raw conformance results,
//! never a judgment folded across them.
//!
//! Every query here is scoped to exactly one run (`run_id`): `agent_snapshots`
//! and `check_results` are both per-run tables, so an agent's rows from two
//! different runs must never be blended into one response.

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppState;
use crate::error::{ApiError, ApiResult};
use crate::routes::runs;

/// The four statuses `check_results.status` is constrained to. Checked
/// against a caller's `status=` filter here, not in SQL, so an invalid value
/// is a clean 400 rather than a query that silently matches nothing.
const VALID_STATUSES: [&str; 4] = ["pass", "fail", "skipped", "error"];

fn validate_status(status: &str) -> ApiResult<()> {
    if VALID_STATUSES.contains(&status) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "invalid status '{status}' — must be one of {}",
            VALID_STATUSES.join(", ")
        )))
    }
}

fn validate_rung(rung: i16) -> ApiResult<()> {
    if (1..=7).contains(&rung) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "invalid rung '{rung}' — must be between 1 and 7"
        )))
    }
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub run: Option<Uuid>,
    pub chain: Option<String>,
    pub rung: Option<i16>,
    pub status: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    100
}

/// One rung's bare status for an agent in the directory — no evidence (that
/// is the detail endpoint's job), just the shape a directory row needs to
/// show all seven questions side by side. A rung this run never asked is
/// simply absent from the vec, same as everywhere else in this schema.
#[derive(Debug, Serialize)]
pub struct RungStatus {
    pub rung: i16,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct AgentListItem {
    pub chain: String,
    pub agent_id: i64,
    pub owner: String,
    pub agent_uri: String,
    pub block_number: i64,
    pub observed_at: DateTime<Utc>,
    pub rungs: Vec<RungStatus>,
}

#[derive(sqlx::FromRow)]
struct SnapshotIdRow {
    chain: String,
    agent_id: i64,
    owner: String,
    agent_uri: String,
    block_number: i64,
    observed_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct RungStatusRow {
    agent_id: i64,
    rung: i16,
    name: String,
    status: String,
}

/// Where this page sits in the whole result set — `total` is what lets a UI
/// render "page 3 of 15" rather than guessing whether a next page exists.
#[derive(Debug, Serialize)]
pub struct PageMeta {
    pub limit: i64,
    pub offset: i64,
    pub total: i64,
}

#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: PageMeta,
}

/// `GET /api/agents?run=&chain=&rung=&status=&limit=&offset=` — the
/// directory, one page at a time. `rung`+`status` filter to "agents failing
/// rung 4"; `run` defaults to the latest completed run (never an in-flight
/// one, whose counts are still changing).
pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> ApiResult<Json<Page<AgentListItem>>> {
    let chain = params
        .chain
        .map(|c| c.trim().to_lowercase())
        .filter(|c| !c.is_empty());
    if let Some(rung) = params.rung {
        validate_rung(rung)?;
    }
    if let Some(status) = &params.status {
        validate_status(status)?;
    }
    let run_id = match params.run {
        Some(id) => id,
        None => runs::latest_completed(&state.db, chain.as_deref()).await?,
    };
    let limit = params.limit.clamp(1, 500);
    let offset = params.offset.max(0);

    // The rung/status filter is an EXISTS against `check_results` — it
    // selects WHICH agents appear, it never folds their rows into a count
    // per agent. `idx_check_results_lookup` (run_id, chain, agent_id) backs
    // the correlated subquery.
    let filter_sql = "WHERE s.run_id = $1 \
         AND ($2::text IS NULL OR s.chain = $2) \
         AND ( \
           ($3::smallint IS NULL AND $4::text IS NULL) \
           OR EXISTS ( \
             SELECT 1 FROM check_results c \
             WHERE c.run_id = s.run_id AND c.chain = s.chain AND c.agent_id = s.agent_id \
               AND ($3::smallint IS NULL OR c.rung = $3) \
               AND ($4::text IS NULL OR c.status = $4) \
           ) \
         )";

    let items_sql = format!(
        "SELECT s.chain, s.agent_id, s.owner, s.agent_uri, s.block_number, s.observed_at \
         FROM agent_snapshots s {filter_sql} \
         ORDER BY s.chain, s.agent_id \
         LIMIT $5 OFFSET $6"
    );
    let rows: Vec<SnapshotIdRow> = sqlx::query_as(&items_sql)
        .bind(run_id)
        .bind(&chain)
        .bind(params.rung)
        .bind(&params.status)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await?;

    let total_sql = format!("SELECT count(*) FROM agent_snapshots s {filter_sql}");
    let total: i64 = sqlx::query_scalar(&total_sql)
        .bind(run_id)
        .bind(&chain)
        .bind(params.rung)
        .bind(&params.status)
        .fetch_one(&state.db)
        .await?;

    // One extra query for the page's rung statuses, rather than joining
    // check_results directly into the paginated query above: a join would
    // multiply each agent's snapshot row by however many rungs it has,
    // breaking LIMIT/OFFSET and the count(*) both.
    let agent_ids: Vec<i64> = rows.iter().map(|r| r.agent_id).collect();
    let rung_rows: Vec<RungStatusRow> = if agent_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(
            "SELECT agent_id, rung, name, status FROM check_results \
             WHERE run_id = $1 AND agent_id = ANY($2) \
             ORDER BY agent_id, rung",
        )
        .bind(run_id)
        .bind(&agent_ids)
        .fetch_all(&state.db)
        .await?
    };

    let items = rows
        .into_iter()
        .map(|r| {
            let rungs = rung_rows
                .iter()
                .filter(|rr| rr.agent_id == r.agent_id)
                .map(|rr| RungStatus {
                    rung: rr.rung,
                    name: rr.name.clone(),
                    status: rr.status.clone(),
                })
                .collect();
            AgentListItem {
                chain: r.chain,
                agent_id: r.agent_id,
                owner: r.owner,
                agent_uri: r.agent_uri,
                block_number: r.block_number,
                observed_at: r.observed_at,
                rungs,
            }
        })
        .collect();

    Ok(Json(Page {
        items,
        page: PageMeta {
            limit,
            offset,
            total,
        },
    }))
}

#[derive(Debug, Deserialize)]
pub struct GetParams {
    pub run: Option<Uuid>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SnapshotDetail {
    /// `token_id` is an ERC-721 `uint256` — cast to TEXT in the query below
    /// because it can exceed `i64`.
    pub token_id: String,
    pub owner: String,
    pub agent_uri: String,
    pub block_number: i64,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct RungResult {
    pub rung: i16,
    pub name: String,
    pub status: String,
    pub evidence: serde_json::Value,
    pub checked_at: DateTime<Utc>,
}

/// What the archived HTTP fetch looked like — never the body itself, which
/// can be up to 1 MiB and belongs to the archive, not a JSON response.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ArchiveSummary {
    pub scheme: String,
    pub request_url: Option<String>,
    pub final_url: Option<String>,
    pub http_status: Option<i32>,
    pub content_type: Option<String>,
    pub body_bytes: Option<i32>,
    pub body_sha256: Option<String>,
    pub truncated: bool,
    pub error: Option<String>,
    pub elapsed_ms: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct AgentDetail {
    pub run_id: Uuid,
    pub chain: String,
    pub agent_id: i64,
    pub snapshot: SnapshotDetail,
    /// Every rung this run actually asked, in rung order. A rung with no row
    /// is simply absent — see the module doc on `check_results` (migration
    /// 0008): absence means "not checked", never a synthesised status.
    pub rungs: Vec<RungResult>,
    /// `None` only when no archive row exists at all (should not happen for
    /// a swept agent, since `write_agent` writes all three rows in one
    /// transaction — but a defensive `Option` costs nothing and is honest
    /// about what an absent row means).
    pub archive: Option<ArchiveSummary>,
}

/// `GET /api/agents/{chain}/{id}?run=` — one agent's snapshot, every rung
/// with its evidence, and the archive summary. Defaults to the latest
/// completed run when `run` is omitted.
pub async fn get_one(
    State(state): State<AppState>,
    Path((chain, agent_id)): Path<(String, i64)>,
    Query(params): Query<GetParams>,
) -> ApiResult<Json<AgentDetail>> {
    let chain = chain.trim().to_lowercase();
    let run_id = match params.run {
        Some(id) => id,
        None => runs::latest_completed(&state.db, Some(&chain)).await?,
    };

    let snapshot = sqlx::query_as::<_, SnapshotDetail>(
        "SELECT token_id::text AS token_id, owner, agent_uri, block_number, observed_at \
         FROM agent_snapshots WHERE run_id = $1 AND chain = $2 AND agent_id = $3",
    )
    .bind(run_id)
    .bind(&chain)
    .bind(agent_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let rungs = sqlx::query_as::<_, RungResult>(
        "SELECT rung, name, status, evidence, checked_at FROM check_results \
         WHERE run_id = $1 AND chain = $2 AND agent_id = $3 ORDER BY rung",
    )
    .bind(run_id)
    .bind(&chain)
    .bind(agent_id)
    .fetch_all(&state.db)
    .await?;

    let archive = sqlx::query_as::<_, ArchiveSummary>(
        "SELECT scheme, request_url, final_url, http_status, content_type, \
                body_bytes, body_sha256, truncated, error, elapsed_ms \
         FROM http_archive WHERE run_id = $1 AND chain = $2 AND agent_id = $3",
    )
    .bind(run_id)
    .bind(&chain)
    .bind(agent_id)
    .fetch_optional(&state.db)
    .await?;

    Ok(Json(AgentDetail {
        run_id,
        chain,
        agent_id,
        snapshot,
        rungs,
        archive,
    }))
}
