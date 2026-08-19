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

/// The seven statuses `check_results.status` is constrained to (the DB's own
/// `check_results_status_check` — migration 0020 — is the source of truth;
/// this list must never drift ahead of or behind it). Checked against a
/// caller's `status=` filter here, not in SQL, so an invalid value is a clean
/// 400 rather than a query that silently matches nothing — and so an
/// unrecognised status string can never be guessed at instead of rejected.
///
/// The additions, and the drift that made this comment necessary: `unclaimed`
/// (2026-07-29, rung 5) was added with its migration, and `refused`
/// (2026-08-06, rungs 2 and 6) with its own. **`unprobeable` was not** — it
/// landed in migration 0015 on 2026-08-01 and this list was never widened, so
/// `?status=unprobeable` answered 400 for the one status tens of thousands of
/// agents actually had. Fixed here. A filter that rejects a status the database
/// contains is worse than no filter, because a 400 reads as "you asked wrong"
/// while the honest answer was a page of agents.
const VALID_STATUSES: [&str; 7] = [
    "pass",
    "fail",
    "skipped",
    "error",
    "refused",
    "unclaimed",
    "unprobeable",
];

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

/// `SnapshotIdRow` plus the window total the search branch's single scan
/// carries on every row. Split from `SnapshotIdRow` rather than making
/// `total` an `Option` there, so the browse branch cannot accidentally
/// deserialize a column it never selects.
#[derive(sqlx::FromRow)]
struct SnapshotTotalRow {
    chain: String,
    agent_id: i64,
    owner: String,
    agent_uri: String,
    block_number: i64,
    observed_at: DateTime<Utc>,
    name: Option<String>,
    total: i64,
}

impl From<SnapshotTotalRow> for SnapshotIdRow {
    fn from(r: SnapshotTotalRow) -> Self {
        SnapshotIdRow {
            chain: r.chain,
            agent_id: r.agent_id,
            owner: r.owner,
            agent_uri: r.agent_uri,
            block_number: r.block_number,
            observed_at: r.observed_at,
            name: r.name,
        }
    }
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

/// The directory page: one run's agents, plus — strictly beside them, never
/// mixed in — whatever the registration tail holds that no census has checked.
///
/// **Why a sibling field and not a merged list.** `items` is a statement about
/// one pinned run: every row in it has seven answers (or a documented absence)
/// and a block it was read at. A tail row has none of that. Merging the two
/// would mean `total`, every count derived from this page, and every consumer
/// iterating `items` would silently start mixing measured agents with unmeasured
/// ones — the single failure this whole feature is designed to make structurally
/// impossible. A separate array cannot be iterated by accident.
///
/// `tail` is a fixed, unpaginated head (at most [`TAIL_HEAD`] rows, newest
/// discovery first) filtered by the same `chain` and `q` as the page. It is
/// deliberately not paged: it is a pointer to `/api/tail`, where the full list
/// lives, not a second result set to walk.
#[derive(Debug, Serialize)]
pub struct AgentPage {
    pub items: Vec<AgentListItem>,
    pub page: PageMeta,
    pub tail: Vec<crate::routes::tail::TailAgent>,
}

/// How many tail rows ride along with a directory page.
const TAIL_HEAD: i64 = 10;

/// `GET /api/agents?run=&chain=&rung=&status=&facet=&q=&limit=&offset=` — the
/// directory, one page at a time. `rung`+`status` filter to "agents failing
/// rung 4"; `facet=2:pass,5:pass` ANDs several such conditions; `q` searches
/// names, descriptions and owner prefixes. `run` defaults to the latest
/// completed run (never an in-flight one, whose counts are still changing).
///
/// The response also carries a `tail` array beside `items` — agents the chain
/// has that no census has checked. See [`AgentPage`] for why they travel
/// separately rather than merged, and [`crate::routes::tail`] for what a tail
/// row is.
pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> ApiResult<Json<AgentPage>> {
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

    let total_sql = format!("SELECT count(*) {from_sql} {filter_sql}");
    let (rows, total): (Vec<SnapshotIdRow>, i64) = if q.is_some() {
        // Searching: ONE scan produces both the page and its total, via
        // `count(*) OVER ()`. A corpus-common term ("agent" matches 88% of
        // the BSC run's documents) makes the filter an unavoidable ~4s scan —
        // the OR in `q_match_sql` spans both joined tables, which no index
        // can serve, and at that selectivity a bitmap plan loses to the seq
        // scan anyway. Two queries scanned twice (~8.6s serially, #58; run
        // concurrently they contended for the same cores and 408'd), so the
        // scan happens once. `SET LOCAL work_mem` keeps the relevance sort
        // off disk — the 236k-row sort spilled at the 4MB default, and the
        // LOCAL form ends with the transaction, so nothing else inherits it.
        //
        // This branch is search-only on purpose: the unsearched directory's
        // two queries are index-served and fast, and a window count there
        // would force a full-run scan on every browse page.
        let paged_sql = format!(
            "SELECT s.chain, s.agent_id, s.owner, s.agent_uri, s.block_number, \
                    s.observed_at, d.name, count(*) OVER () AS total \
             {from_sql} {filter_sql} {order_sql} \
             LIMIT $8 OFFSET $9"
        );
        let mut tx = state.db.begin().await?;
        sqlx::query("SET LOCAL work_mem = '64MB'")
            .execute(&mut *tx)
            .await?;
        let paged: Vec<SnapshotTotalRow> = sqlx::query_as(&paged_sql)
            .bind(run_id)
            .bind(&chain)
            .bind(params.rung)
            .bind(&params.status)
            .bind(&facet_rungs)
            .bind(&facet_statuses)
            .bind(&q)
            .bind(limit)
            .bind(offset)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        match paged.first() {
            Some(first) => {
                let total = first.total;
                (paged.into_iter().map(SnapshotIdRow::from).collect(), total)
            }
            // An empty page means the window count never materialized — which
            // is "no matches" only when this is the FIRST page. Past the end
            // (the pagination-overflow probe) the total still exists and must
            // hold, so it is counted the old way on this rare path.
            None if offset > 0 => {
                let total = sqlx::query_scalar(&total_sql)
                    .bind(run_id)
                    .bind(&chain)
                    .bind(params.rung)
                    .bind(&params.status)
                    .bind(&facet_rungs)
                    .bind(&facet_statuses)
                    .bind(&q)
                    .fetch_one(&state.db)
                    .await?;
                (Vec::new(), total)
            }
            None => (Vec::new(), 0),
        }
    } else {
        // Browsing: the page is a pkey-ordered index scan and the count is
        // index-served — two cheap queries, unchanged.
        let items_sql = format!(
            "SELECT s.chain, s.agent_id, s.owner, s.agent_uri, s.block_number, \
                    s.observed_at, d.name \
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
        (rows, total)
    };

    // One extra query for the page's rung statuses, rather than joining
    // check_results directly into the paginated query above: a join would
    // multiply each agent's snapshot row by however many rungs it has,
    // breaking LIMIT/OFFSET and the count(*) both.
    //
    // `chain` is in the predicate because `check_results_unique` is
    // (run_id, chain, agent_id, rung). Without it, this seeks on `run_id`
    // alone and then scans every row the run wrote to test `agent_id` — for
    // the 2026-08 BNB Chain run, 1.76 million rows to return 350. Measured on
    // production, one 50-agent page:
    //
    //   without chain   Parallel Seq Scan    278,731 buffers   8,915 ms
    //   with chain      Bitmap Index Scan        223 buffers      8.8 ms
    //
    // That is the difference between this endpoint answering and returning
    // 408, and it is the same omission that made the `refused` backfill take
    // four hours. `chain` is redundant here — a run has exactly one — which is
    // exactly why it keeps getting left out.
    let agent_ids: Vec<i64> = rows.iter().map(|r| r.agent_id).collect();
    let rung_rows: Vec<RungStatusRow> = if agent_ids.is_empty() {
        Vec::new()
    } else {
        // Every row on this page belongs to one run, and a run is one chain,
        // so the page's own first row names it. Taken from the data rather
        // than from `params.chain`, which is optional and absent whenever the
        // caller addressed a run directly.
        let page_chain = rows[0].chain.clone();
        sqlx::query_as(
            "SELECT agent_id, rung, name, status FROM check_results \
             WHERE run_id = $1 AND chain = $3 AND agent_id = ANY($2) \
             ORDER BY agent_id, rung",
        )
        .bind(run_id)
        .bind(&agent_ids)
        .bind(&page_chain)
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

    // Read AFTER the run-scoped page and kept in its own array. This query
    // touches `registration_tail` only — it cannot alter `items`, `total`, or
    // anything the caller would quote as a census figure.
    let tail =
        crate::routes::tail::matches(&state.db, chain.as_deref(), q.as_deref(), TAIL_HEAD, 0)
            .await?;

    Ok(Json(AgentPage {
        items,
        page: PageMeta {
            limit,
            offset,
            total,
        },
        tail,
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
    /// Always `"census"`. Added so a client has ONE field to branch on across
    /// both shapes this endpoint can return; existing clients that ignore it
    /// are unaffected, because everything else is exactly where it was.
    pub source: &'static str,
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

/// Either answer this endpoint can give, as one response type.
///
/// `#[serde(untagged)]`: each variant serializes as its own object, with no
/// wrapper key. The discriminator is the `source` field INSIDE each shape —
/// `"census"` or `"tail"` — so a caller reads one field at the top level of
/// the body it already has, rather than unwrapping an envelope.
///
/// The two shapes share only `chain` and `agent_id`. A census result has
/// `run_id`, `snapshot` and `rungs`; a tail result has none of those and has
/// no array at all. That is the safety property: a client that ignores
/// `source` and reaches for `rungs` finds nothing there, so it fails loudly
/// instead of rendering seven blank statuses as though the agent had been
/// checked and found wanting.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum AgentDetailResponse {
    /// One agent as a pinned run measured it. `Box`ed because this variant is
    /// far larger than the other, and clippy is right that a lopsided enum
    /// makes every value pay for the big one.
    Census(Box<AgentDetail>),
    /// One agent the chain has and no census has checked.
    Tail(crate::routes::tail::TailAgent),
}

/// `GET /api/agents/{chain}/{id}?run=` — one agent's snapshot, every rung
/// with its evidence, and the archive summary. Defaults to the latest
/// completed run when `run` is omitted.
///
/// **The tail fallback.** When the run has no row for this agent, the
/// registration tail is asked before giving up. This is the fix for a real
/// and predictable failure: an agent minted after the last sweep 404s here,
/// and the person most likely to click that link is the registrant, minutes
/// after minting, who reasonably concludes the site is broken.
///
/// What comes back in that case is NOT a census result wearing a thin
/// disguise. It is a different shape ([`AgentDetailResponse::Tail`]) with a
/// `"source": "tail"` discriminator, no `run_id` — there is no run to cite —
/// and no rungs array of any kind, because no check was run. A 404 is still
/// the answer when neither table knows the id.
///
/// Only UNSWEPT tail rows answer here. Once a census has covered an id, the
/// tail row is marked superseded and this endpoint's run-scoped path is the
/// only one that can describe it — asking `?run=` for an older run that
/// predates the agent correctly 404s rather than quietly substituting a
/// receipt for a measurement.
pub async fn get_one(
    State(state): State<AppState>,
    Path((chain, agent_id)): Path<(String, i64)>,
    Query(params): Query<GetParams>,
) -> ApiResult<Json<AgentDetailResponse>> {
    let chain = chain.trim().to_lowercase();
    // Resolved as an `Option`, not with `?`. A chain that has never had a
    // completed run has no census to look in — but it can still have a tail,
    // and that is exactly the case (a newly enabled chain) where a 404 for
    // every agent would be most misleading.
    let run_id = match params.run {
        Some(id) => Some(id),
        None => match runs::latest_completed(&state.db, Some(&chain)).await {
            Ok(id) => Some(id),
            Err(ApiError::NotFound) => None,
            Err(e) => return Err(e),
        },
    };

    let snapshot =
        match run_id {
            Some(run_id) => sqlx::query_as::<_, SnapshotDetail>(
                "SELECT token_id::text AS token_id, owner, agent_uri, block_number, observed_at \
                 FROM agent_snapshots WHERE run_id = $1 AND chain = $2 AND agent_id = $3",
            )
            .bind(run_id)
            .bind(&chain)
            .bind(agent_id)
            .fetch_optional(&state.db)
            .await?,
            None => None,
        };

    let (Some(run_id), Some(snapshot)) = (run_id, snapshot) else {
        // No census row. Ask the tail before 404ing — but ONLY when the caller
        // did not pin a run.
        //
        // `?run=` is a request for one measurement, at one block. Answering it
        // from the tail would hand back a receipt for an agent that did not
        // exist when that run was taken, and a caller who pinned a run is
        // precisely the caller who must not be given data from outside it. A
        // run id that names nothing must 404 for the same reason: silently
        // widening the question is how a pinned figure stops meaning anything.
        //
        // Without the pin the question is "what do you know about this
        // agent", and the tail is a legitimate answer to it.
        if params.run.is_some() {
            return Err(ApiError::NotFound);
        }
        return match crate::routes::tail::lookup(&state.db, &chain, agent_id).await? {
            Some(t) => Ok(Json(AgentDetailResponse::Tail(t))),
            None => Err(ApiError::NotFound),
        };
    };

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

    Ok(Json(AgentDetailResponse::Census(Box::new(AgentDetail {
        source: "census",
        run_id,
        chain,
        agent_id,
        name,
        description,
        snapshot,
        rungs,
        archive,
    }))))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every status the checker can produce must be a status this endpoint
    /// will accept as a filter.
    ///
    /// The `match` below has no wildcard arm on purpose: adding a variant to
    /// `checks::CheckStatus` breaks this file's compilation, which is the only
    /// mechanism that would have caught `unprobeable` going missing from
    /// `VALID_STATUSES` for five days. A doc comment asking the next person to
    /// remember did not.
    #[test]
    fn every_status_the_checker_can_produce_is_queryable() {
        use checks::CheckStatus::*;
        for status in [Pass, Fail, Skipped, Error, Refused, Unclaimed, Unprobeable] {
            match status {
                Pass | Fail | Skipped | Error | Refused | Unclaimed | Unprobeable => {}
            }
            assert!(
                validate_status(status.as_str()).is_ok(),
                "`{}` is a status the database can hold, and this endpoint rejects it",
                status.as_str()
            );
        }
        assert_eq!(VALID_STATUSES.len(), 7);
    }

    #[test]
    fn an_invented_status_is_still_rejected() {
        for junk in ["refuse", "REFUSED", "live", ""] {
            assert!(validate_status(junk).is_err(), "{junk:?}");
        }
    }
}
