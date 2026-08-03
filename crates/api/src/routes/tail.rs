//! The registration tail — agents the chain has, that no census has checked.
//!
//! ## Why this is a separate shape, not a thin census row
//!
//! Everything else this API serves is a run's answer to a question. A tail row
//! is not: it is a receipt saying "the registry contained this id at this
//! block", produced by `sweeper`'s `tail` binary between censuses (migration
//! 0018). Nothing was fetched, nothing was judged, and there is no run to cite.
//!
//! That difference is enforced in the wire format rather than left to a
//! caller's discipline. A tail response carries a `"source": "tail"`
//! discriminator AND shares no field name with a census response beyond
//! `chain` and `agent_id`: no `run_id`, no `snapshot`, no `rungs` — not an
//! empty `rungs` array, no array at all. A client that ignores the
//! discriminator therefore cannot render a tail agent as a census result: the
//! fields it would read are simply absent, so it breaks visibly instead of
//! displaying seven statuses for an agent that has none. An empty `rungs: []`
//! would have been the dangerous shape, because "all seven checks are missing"
//! renders indistinguishably from "all seven checks failed" in most UIs.
//!
//! ## What a tail query can and cannot match
//!
//! Owner-address prefix, and an exact agent id. NOT name or description —
//! those live in `agent_documents`, which is written by a sweep that fetched
//! and parsed the document. The tail fetched nothing, so it has no name to
//! match, and inventing one from the URI would be a claim about a document
//! nobody has read.
//!
//! ## Superseded rows are invisible here
//!
//! Every query below filters `superseded_by_run IS NULL`. Once a census run
//! has swept an id, that agent has real answers and belongs to the run-scoped
//! endpoints; the tail row survives only as the record of when the agent first
//! appeared, which the census — a series of pinned snapshots — cannot express.

use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::ApiResult;

/// The columns every tail read returns, in one place so the detail lookup, the
/// directory sibling and the dedicated endpoint cannot drift apart.
const TAIL_COLUMNS: &str = "chain, agent_id, token_id::text AS token_id, owner, agent_uri, \
                            discovery_block, discovered_at";

/// One agent the chain has and the census has not checked.
///
/// `source` is a constant `"tail"`, serialized on every response so a client
/// can branch on one field. `checks_available` is deliberately a boolean and
/// not an empty list: there is no array here to iterate, and a `false` says
/// what an absent array only implies.
#[derive(Debug, Serialize)]
pub struct TailAgent {
    /// Always `"tail"`. Set in code rather than read from the row — it
    /// describes which table answered, not anything stored in it.
    pub source: &'static str,
    pub chain: String,
    pub agent_id: i64,
    /// `token_id` is an ERC-721 `uint256`, cast to TEXT in the query because
    /// it can exceed `i64` — same as the census detail endpoint.
    pub token_id: String,
    pub owner: String,
    pub agent_uri: String,
    /// The block `ownerOf` and `tokenURI` were both read at. A tick's own pin,
    /// not a census pin: it makes this row's two values simultaneous with each
    /// other, and says nothing about any other agent.
    pub discovery_block: i64,
    /// When this id was FIRST seen by the poller. Not a registration time —
    /// the tail does not read logs, so it knows when it looked, not when the
    /// mint happened.
    pub discovered_at: DateTime<Utc>,
    /// Always `false`. No document was fetched and no rung was answered for
    /// this agent, so there is nothing to show and no status to infer.
    pub checks_available: bool,
}

fn tail_source() -> &'static str {
    "tail"
}

/// The row as the database hands it over. Kept separate from [`TailAgent`] so
/// the two constant fields (`source`, `checks_available`) are stamped in code,
/// once, and cannot be made to say anything else by a stray column alias.
#[derive(Debug, sqlx::FromRow)]
struct TailRow {
    chain: String,
    agent_id: i64,
    token_id: String,
    owner: String,
    agent_uri: String,
    discovery_block: i64,
    discovered_at: DateTime<Utc>,
}

impl From<TailRow> for TailAgent {
    fn from(r: TailRow) -> Self {
        TailAgent {
            source: tail_source(),
            chain: r.chain,
            agent_id: r.agent_id,
            token_id: r.token_id,
            owner: r.owner,
            agent_uri: r.agent_uri,
            discovery_block: r.discovery_block,
            discovered_at: r.discovered_at,
            checks_available: false,
        }
    }
}

/// An exact agent id typed into a search box, if that is what `q` is.
///
/// Owner addresses and ids are the only two things a tail row can be matched
/// on, and they are trivially distinguishable — one is decimal digits, the
/// other is `0x…`. Parsed in Rust rather than cast in SQL so a non-numeric `q`
/// is simply "not an id" instead of a query that errors.
fn q_as_agent_id(q: &str) -> Option<i64> {
    q.parse::<i64>().ok().filter(|&n| n >= 0)
}

/// One agent, if the tail knows it and no census has swept it yet.
///
/// The fallback behind `GET /api/agents/{chain}/{id}` — see
/// [`crate::routes::agents::get_one`] for why a 404 for a freshly minted agent
/// is the bug this closes.
pub async fn lookup(
    pool: &sqlx::PgPool,
    chain: &str,
    agent_id: i64,
) -> ApiResult<Option<TailAgent>> {
    let row: Option<TailRow> = sqlx::query_as(&format!(
        "SELECT {TAIL_COLUMNS} FROM registration_tail \
         WHERE chain = $1 AND agent_id = $2 AND superseded_by_run IS NULL"
    ))
    .bind(chain)
    .bind(agent_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(TailAgent::from))
}

/// The tail rows matching a directory query, newest discovery first.
///
/// `q` is optional: with none, this is "what has appeared since the last
/// census", which is the useful default for a chain-scoped directory page.
pub async fn matches(
    pool: &sqlx::PgPool,
    chain: Option<&str>,
    q: Option<&str>,
    limit: i64,
    offset: i64,
) -> ApiResult<Vec<TailAgent>> {
    let rows: Vec<TailRow> = sqlx::query_as(&format!(
        "SELECT {TAIL_COLUMNS} FROM registration_tail \
         WHERE superseded_by_run IS NULL \
           AND ($1::text IS NULL OR chain = $1) \
           AND ($2::text IS NULL \
                OR owner LIKE lower($2) || '%' \
                OR ($3::bigint IS NOT NULL AND agent_id = $3)) \
         ORDER BY discovered_at DESC, chain, agent_id \
         LIMIT $4 OFFSET $5"
    ))
    .bind(chain)
    .bind(q)
    .bind(q.and_then(q_as_agent_id))
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(TailAgent::from).collect())
}

/// How many tail rows match, for the paged endpoint's `total`.
async fn match_count(pool: &sqlx::PgPool, chain: Option<&str>, q: Option<&str>) -> ApiResult<i64> {
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM registration_tail \
         WHERE superseded_by_run IS NULL \
           AND ($1::text IS NULL OR chain = $1) \
           AND ($2::text IS NULL \
                OR owner LIKE lower($2) || '%' \
                OR ($3::bigint IS NOT NULL AND agent_id = $3))",
    )
    .bind(chain)
    .bind(q)
    .bind(q.and_then(q_as_agent_id))
    .fetch_one(pool)
    .await?;
    Ok(total)
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub chain: Option<String>,
    pub q: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

/// The paged tail listing. Deliberately its own envelope rather than the
/// directory's `items`/`page` shape reused: a caller holding this response
/// should not be able to hand it to code that expects census rows.
#[derive(Debug, Serialize)]
pub struct TailPage {
    /// Always `"tail"`, for the same reason each item carries it: one field to
    /// branch on, at the level a caller actually looks.
    pub source: &'static str,
    pub tail: Vec<TailAgent>,
    pub limit: i64,
    pub offset: i64,
    pub total: i64,
}

/// `GET /api/tail?chain=&q=&limit=&offset=` — everything the chain has that no
/// census has checked yet.
///
/// **Why a dedicated endpoint rather than folding results into
/// `/api/search`.** That endpoint returns a JSON ARRAY of per-run groups at
/// the top level, so there is nowhere to hang a sibling field without changing
/// its shape for every existing client — and the alternative, a synthetic
/// group with a made-up `run_id`, is exactly the lie this whole design exists
/// to avoid. `/api/agents` does have an envelope, and there the tail travels
/// as a sibling `tail` array beside `items` (never merged into it). A caller
/// searching across runs makes one extra request to this endpoint and keeps
/// two lists that are never confusable.
pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> ApiResult<Json<TailPage>> {
    let chain = params
        .chain
        .map(|c| c.trim().to_lowercase())
        .filter(|c| !c.is_empty());
    let q = params
        .q
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let limit = params.limit.clamp(1, 200);
    let offset = params.offset.max(0);

    let tail = matches(&state.db, chain.as_deref(), q.as_deref(), limit, offset).await?;
    let total = match_count(&state.db, chain.as_deref(), q.as_deref()).await?;
    Ok(Json(TailPage {
        source: tail_source(),
        tail,
        limit,
        offset,
        total,
    }))
}

/// One chain's tail, in the two numbers a page header actually shows.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ChainTailSummary {
    pub chain: String,
    /// Rows no census run has swept yet. Never added to a population figure:
    /// the census's count is the pinned run's, and this is a count of things
    /// that run did not see.
    pub unswept: i64,
    /// The newest discovery. `None` when the tail is empty, which is the
    /// healthy steady state right after a sweep — not a missing value.
    pub newest_discovered_at: Option<DateTime<Utc>>,
    /// When the poller last looked at this chain, and how far it had got.
    /// `None` means it has never polled this chain — which is the difference
    /// between "nothing new" and "nobody is looking", and a site that shows
    /// the first when the second is true is lying quietly.
    pub polled_at: Option<DateTime<Utc>>,
    pub cursor_agent_id: Option<i64>,
    pub cursor_block: Option<i64>,
}

/// `GET /api/tail/summary` — per chain, how many unswept agents the tail holds
/// and when it last saw one.
///
/// Driven by `chains`, so a chain with an empty tail appears with `0` rather
/// than vanishing: "no new agents since the sweep" is an answer, and its
/// absence would be indistinguishable from a chain nobody is polling.
pub async fn summary(State(state): State<AppState>) -> ApiResult<Json<Vec<ChainTailSummary>>> {
    let rows: Vec<ChainTailSummary> = sqlx::query_as(
        "SELECT c.chain, \
                coalesce(t.unswept, 0) AS unswept, \
                t.newest_discovered_at, \
                cur.polled_at, \
                cur.highest_agent_id AS cursor_agent_id, \
                cur.last_block AS cursor_block \
           FROM chains c \
           LEFT JOIN ( \
             SELECT chain, count(*) AS unswept, max(discovered_at) AS newest_discovered_at \
               FROM registration_tail WHERE superseded_by_run IS NULL GROUP BY chain \
           ) t ON t.chain = c.chain \
           LEFT JOIN registration_tail_cursor cur ON cur.chain = c.chain \
          WHERE c.enabled \
          ORDER BY c.chain",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> TailAgent {
        TailAgent {
            source: tail_source(),
            chain: "base".into(),
            agent_id: 60_123,
            token_id: "60123".into(),
            owner: "0x1111111111111111111111111111111111111111".into(),
            agent_uri: "https://example.test/agent.json".into(),
            discovery_block: 41_900_123,
            discovered_at: DateTime::from_timestamp(1_800_000_000, 0).unwrap(),
            checks_available: false,
        }
    }

    /// **The deliverable assertion.** A tail response must carry no checks
    /// array under ANY name the census uses — not `rungs`, not `checks`, not
    /// an empty one. An empty array is the dangerous shape: "seven statuses
    /// missing" and "seven statuses failed" render the same way in most UIs,
    /// and the whole point of this endpoint is that a tail agent has not been
    /// judged at all.
    #[test]
    fn a_tail_response_has_no_checks_array_of_any_kind() {
        let v = serde_json::to_value(row()).unwrap();
        let obj = v.as_object().unwrap();
        assert!(obj.get("rungs").is_none(), "tail must not carry rungs");
        assert!(obj.get("checks").is_none(), "tail must not carry checks");
        assert!(
            !obj.values().any(|x| x.is_array()),
            "no array of any name may appear in a tail response: {v}"
        );
        assert_eq!(obj["checks_available"], serde_json::json!(false));
    }

    /// An old client that ignores `source` must not be able to read a tail
    /// response as a census one. It cannot: every field it would reach for is
    /// absent, so it fails visibly rather than rendering a fabricated result.
    #[test]
    fn a_tail_response_shares_no_field_with_a_census_detail_beyond_identity() {
        let v = serde_json::to_value(row()).unwrap();
        let obj = v.as_object().unwrap();
        for census_only in [
            "run_id",
            "snapshot",
            "rungs",
            "archive",
            "name",
            "description",
        ] {
            assert!(
                obj.get(census_only).is_none(),
                "{census_only} belongs to a census result and must never appear on a tail row"
            );
        }
        assert_eq!(obj["source"], serde_json::json!("tail"));
        // Identity is shared on purpose — it is the same agent.
        assert!(obj.contains_key("chain") && obj.contains_key("agent_id"));
    }

    /// A numeric `q` is an agent id; an address, or anything else, is not.
    #[test]
    fn only_a_plain_number_is_read_as_an_agent_id() {
        assert_eq!(q_as_agent_id("60123"), Some(60_123));
        assert_eq!(q_as_agent_id("0x1111"), None);
        assert_eq!(q_as_agent_id("weather bot"), None);
        assert_eq!(q_as_agent_id("-1"), None);
        assert_eq!(q_as_agent_id(""), None);
    }
}
