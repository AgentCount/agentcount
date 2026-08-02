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

/// The five statuses `check_results.status` is constrained to (the DB's own
/// `check_results_status_check` — migration 0011 — is the source of truth;
/// this list must never drift ahead of or behind it). `unclaimed` was added
/// 2026-07-29 (P0 FIX 4/5 addendum): rung 5 (`bound`) produces it for a
/// document that made no binding claim to verify. Checked against a caller's
/// `status=` filter here, not in SQL, so an invalid value is a clean 400
/// rather than a query that silently matches nothing — and so an
/// unrecognised status string can never be guessed at instead of rejected.
const VALID_STATUSES: [&str; 5] = ["pass", "fail", "skipped", "error", "unclaimed"];

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
    /// Comma-separated `rung:status` pairs, ANDed: `facet=2:pass,5:pass,7:pass`
    /// selects agents that pass rungs 2, 5 AND 7 — the query no other tool in
    /// the ecosystem can answer, and the reason the directory exists.
    ///
    /// Deliberately a NEW parameter rather than making `rung`/`status`
    /// repeatable: those two stay exactly as they were, so every existing link
    /// and the web repo's `check-api.ts` keep working unchanged. Comma-
    /// separated rather than a repeated key because a repeated key needs
    /// `axum_extra`'s `Query` — a dependency for a wire format that is no more
    /// linkable than this one.
    pub facet: Option<String>,
    /// Free-text search over the document's `name` and `description`, or an
    /// owner-address prefix. See [`q_match_sql`] for why the owner is
    /// matched by prefix rather than folded into the full-text vector.
    pub q: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    100
}

/// A rung/status pair from `facet=`. Validated against the same
/// `validate_rung`/`validate_status` the scalar form uses, so an invalid facet
/// is a clean 400 naming the offending value rather than a query that silently
/// matches nothing.
fn parse_facets(raw: &str) -> ApiResult<Vec<(i16, String)>> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (rung_str, status) = part.split_once(':').ok_or_else(|| {
            ApiError::BadRequest(format!(
                "invalid facet '{part}' — expected the form <rung>:<status>, e.g. 2:pass"
            ))
        })?;
        let rung: i16 = rung_str.trim().parse().map_err(|_| {
            ApiError::BadRequest(format!("invalid facet rung '{rung_str}' — must be 1 to 7"))
        })?;
        validate_rung(rung)?;
        let status = status.trim().to_string();
        validate_status(&status)?;
        out.push((rung, status));
    }
    Ok(out)
}

/// The one definition of what `q` matches, over the aliases every caller
/// binds: `s` = `agent_snapshots`, `d` = `agent_documents` (LEFT JOINed on
/// run/chain/agent). Three OR'd forms: full text over name+description (the
/// generated `search` column, GIN-indexed), trigram similarity on the name
/// (the "typed it slightly wrong" case), and an owner-address prefix — prefix
/// rather than folded into the full-text vector because an address is one
/// 42-char token nobody types in full, and `LIKE 'prefix%'` is what
/// `idx_snapshots_owner` can serve.
///
/// The agent's on-chain `agentWallet` is deliberately NOT matched: no table
/// stores it (rung 1 evidence carries `ownerOf`, not `getAgentWallet` — see
/// `analysis/payments-per-chain.md` on why it cannot be reconstructed from
/// events either). Matching it means a sweep-time `getAgentWallet` read into
/// a real column first, not a scan of evidence JSONB here.
///
/// `p` is the SQL placeholder (e.g. `"$7"`), passed in because the two
/// endpoints that share this fragment bind it at different positions. Shared
/// verbatim by `/api/agents` and `/api/search` so "a match" can never mean
/// two different things depending on which box the text was typed into.
pub(crate) fn q_match_sql(p: &str) -> String {
    format!(
        "({p}::text IS NULL \
          OR d.search @@ plainto_tsquery('simple', {p}) \
          OR d.name % {p} \
          OR s.owner LIKE lower({p}) || '%')"
    )
}

/// The relevance both search-ordered queries sort by. Negated because ASC is
/// the only direction that keeps `/api/agents`' NULL-search branch (a
/// constant 0) sorting consistently, and `/api/search` inherits the sign so
/// the two endpoints rank identically.
pub(crate) fn q_relevance_sql(p: &str) -> String {
    format!("-similarity(coalesce(d.name, ''), {p})")
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
    /// The `name` the document declared, projected into `agent_documents` by
    /// migration 0012. `None` when the document had no usable name or never
    /// parsed — never an empty string, and never a synthesised "Agent #N",
    /// which is the frontend's fallback to render, not a fact to store.
    pub name: Option<String>,
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
    name: Option<String>,
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

/// `GET /api/agents?run=&chain=&rung=&status=&facet=&q=&limit=&offset=` — the
/// directory, one page at a time. `rung`+`status` filter to "agents failing
/// rung 4"; `facet=2:pass,5:pass` ANDs several such conditions; `q` searches
/// names, descriptions and owner prefixes. `run` defaults to the latest
/// completed run (never an in-flight one, whose counts are still changing).
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

    let facets = match &params.facet {
        Some(raw) => parse_facets(raw)?,
        None => Vec::new(),
    };
    // Bound as two parallel arrays so the parameter COUNT is fixed however
    // many facets arrive — the alternative, splicing one `EXISTS` per facet
    // into the SQL string, would make the placeholder numbering depend on user
    // input, which is exactly where injection bugs live.
    let (facet_rungs, facet_statuses): (Vec<i16>, Vec<String>) = facets.into_iter().unzip();
    let facet_rungs = (!facet_rungs.is_empty()).then_some(facet_rungs);
    let facet_statuses = (!facet_statuses.is_empty()).then_some(facet_statuses);

    let q = params
        .q
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // `agent_documents` is LEFT JOINed rather than subqueried because the name
    // is needed in the response anyway — a rung-2 failure has no document and
    // so no row here, which is precisely a NULL name, not a missing agent.
    let from_sql = "FROM agent_snapshots s \
         LEFT JOIN agent_documents d \
           ON d.run_id = s.run_id AND d.chain = s.chain AND d.agent_id = s.agent_id";

    // Three independent filters, ANDed:
    //
    // 1. The original scalar rung/status pair ($3/$4), untouched.
    // 2. The facet array ($5/$6): every (rung, status) pair must have a
    //    matching `check_results` row. Counting matched facets and requiring
    //    the count to equal `cardinality` is what makes this AND rather than
    //    OR. `idx_check_results_lookup` backs the correlated subquery.
    // 3. Free text ($7) — the shared `q_match_sql` fragment, spliced in by
    //    `format!` below.
    //
    // None of these folds a per-agent count into a verdict — they select WHICH
    // agents appear, never how well any one of them did.
    let filter_head = "WHERE s.run_id = $1 \
         AND ($2::text IS NULL OR s.chain = $2) \
         AND ( \
           ($3::smallint IS NULL AND $4::text IS NULL) \
           OR EXISTS ( \
             SELECT 1 FROM check_results c \
             WHERE c.run_id = s.run_id AND c.chain = s.chain AND c.agent_id = s.agent_id \
               AND ($3::smallint IS NULL OR c.rung = $3) \
               AND ($4::text IS NULL OR c.status = $4) \
           ) \
         ) \
         AND ( \
           $5::smallint[] IS NULL \
           OR ( \
             SELECT count(*) FROM unnest($5::smallint[], $6::text[]) AS f(rung, status) \
             WHERE EXISTS ( \
               SELECT 1 FROM check_results c \
               WHERE c.run_id = s.run_id AND c.chain = s.chain AND c.agent_id = s.agent_id \
                 AND c.rung = f.rung AND c.status = f.status \
             ) \
           ) = cardinality($5::smallint[]) \
         )";
    let filter_sql = format!("{filter_head} AND {}", q_match_sql("$7"));

    // Relevance first when searching, and only then — an unsearched directory
    // stays in the stable (chain, agent_id) order that makes pagination
    // reproducible. See `q_relevance_sql` for why the similarity is negated.
    let order_sql = format!(
        "ORDER BY \
         CASE WHEN $7::text IS NULL THEN 0::real \
              ELSE {} END, \
         s.chain, s.agent_id",
        q_relevance_sql("$7")
    );

    let items_sql = format!(
        "SELECT s.chain, s.agent_id, s.owner, s.agent_uri, s.block_number, s.observed_at, d.name \
         {from_sql} {filter_sql} {order_sql} \
         LIMIT $8 OFFSET $9"
    );
    let rows: Vec<SnapshotIdRow> = sqlx::query_as(&items_sql)
        .bind(run_id)
        .bind(&chain)
        .bind(params.rung)
        .bind(&params.status)
        .bind(&facet_rungs)
        .bind(&facet_statuses)
        .bind(&q)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await?;

    let total_sql = format!("SELECT count(*) {from_sql} {filter_sql}");
    let total: i64 = sqlx::query_scalar(&total_sql)
        .bind(run_id)
        .bind(&chain)
        .bind(params.rung)
        .bind(&params.status)
        .bind(&facet_rungs)
        .bind(&facet_statuses)
        .bind(&q)
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
                name: r.name,
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
    /// From `agent_documents` (migration 0012) — what the document called
    /// itself. `None` when it declared no usable name or never parsed. This is
    /// identity for display, not evidence: nothing about a rung's status
    /// depends on it.
    pub name: Option<String>,
    pub description: Option<String>,
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

    // A LEFT-JOIN-shaped lookup done separately: an agent whose document never
    // resolved has no `agent_documents` row at all, and `fetch_optional`
    // renders that as two `None`s rather than a 404 — the agent exists, its
    // document did not.
    let document: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT name, description FROM agent_documents \
         WHERE run_id = $1 AND chain = $2 AND agent_id = $3",
    )
    .bind(run_id)
    .bind(&chain)
    .bind(agent_id)
    .fetch_optional(&state.db)
    .await?;
    let (name, description) = document.unwrap_or((None, None));

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
        name,
        description,
        snapshot,
        rungs,
        archive,
    }))
}
