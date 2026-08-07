//! Cross-run search — one `q`, several runs, matches grouped per run.
//!
//! The census now spans several chains, and each chain's canonical facts live
//! in a different run. A search box scoped to one run (which is what
//! `/api/agents?q=` is) silently answers "nothing" about every agent on the
//! other chains. This endpoint asks the SAME question of several runs at once
//! and keeps the answers separate: one group per run, never a blended list —
//! the module rule that rows from two runs must not be mixed into one result
//! holds here too, the groups just travel in one response.
//!
//! WHY THE CALLER NAMES THE RUNS. This API has no notion of a "published" or
//! "canonical" run — that editorial choice lives in the web repo's
//! `published-runs.json`, in git, next to the reports that cite those runs.
//! Inventing a server-side default (say, latest completed per chain) would be
//! a second, silently diverging definition of canonical. So the caller passes
//! its run set explicitly, capped at [`MAX_RUNS`], and the groups come back in
//! the caller's order — presentation stays the caller's decision too.
//!
//! Match semantics are `agents::q_match_sql`, the exact fragment
//! `/api/agents?q=` uses — including its note on why the on-chain
//! `agentWallet` is not (yet) part of a match.

use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppState;
use crate::error::{ApiError, ApiResult};
use crate::routes::agents::{AgentListItem, RungStatus, q_match_sql, q_relevance_sql};

/// Upper bound on `runs=`. The canonical set is one run per chain, so 16 is
/// years of chain growth away — the cap exists so an arbitrary caller cannot
/// fan one request out into an unbounded number of per-run workloads.
const MAX_RUNS: usize = 16;

/// Matches returned per run. This endpoint answers "where does this name
/// live?" — the directory, filtered to that run, is where the full result set
/// (with paging) already exists, so `total` plus a short head is the whole
/// job here.
const ITEMS_PER_RUN: i64 = 5;

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    /// Required. Same semantics as `/api/agents?q=`.
    pub q: Option<String>,
    /// Required. Comma-separated run UUIDs, at most [`MAX_RUNS`], duplicates
    /// collapsed. Comma-separated for the same reason `facet=` is: a repeated
    /// key needs `axum_extra`'s `Query`, a dependency for a wire format no
    /// more linkable than this one.
    pub runs: Option<String>,
}

/// One run's slice of the answer. `total` counts every match in the run;
/// `items` is the [`ITEMS_PER_RUN`]-capped head, ranked by the same relevance
/// `/api/agents?q=` uses. A run with no matches still gets its group — the
/// caller asked about that run, and "0 on this chain" is an answer, not an
/// omission.
#[derive(Debug, Serialize)]
pub struct RunGroup {
    pub run_id: Uuid,
    pub chain: String,
    pub total: i64,
    pub items: Vec<AgentListItem>,
}

/// The run-id list from `runs=`. Order-preserving and de-duplicating: the
/// response groups come back in this order, and a run named twice must not be
/// searched (or reported) twice.
fn parse_runs(raw: &str) -> ApiResult<Vec<Uuid>> {
    let mut out: Vec<Uuid> = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let id: Uuid = part.parse().map_err(|_| {
            ApiError::BadRequest(format!("invalid run id '{part}' — must be a UUID"))
        })?;
        if !out.contains(&id) {
            out.push(id);
        }
    }
    if out.is_empty() {
        return Err(ApiError::BadRequest(
            "runs must name at least one run id".into(),
        ));
    }
    if out.len() > MAX_RUNS {
        return Err(ApiError::BadRequest(format!(
            "too many runs — at most {MAX_RUNS} per request"
        )));
    }
    Ok(out)
}

#[derive(sqlx::FromRow)]
struct MatchRow {
    run_id: Uuid,
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
    run_id: Uuid,
    agent_id: i64,
    rung: i16,
    name: String,
    status: String,
}

/// `GET /api/search?q=<text>&runs=<uuid>,<uuid>,…` — the shared `q` match
/// over every named run, grouped per run. Both parameters are required; an
/// empty `q` or an empty `runs` is a 400, not an everything-matches query.
///
/// A requested run id the `runs` table does not know is dropped from the
/// response rather than failing the whole search: the caller's canonical set
/// is maintained in another repo and may briefly name a run this database
/// has not seen — the other chains' answers should not be held hostage to
/// that skew. The caller can detect the drop by comparing group count to
/// request count.
pub async fn get(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<Vec<RunGroup>>> {
    let q = params
        .q
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest("q must not be empty".into()))?;
    let run_ids = parse_runs(params.runs.as_deref().unwrap_or(""))?;

    // Which requested runs exist, and each one's chain — a run is one chain's
    // sweep, so the group's `chain` is the run's, not derived from its rows.
    let known: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT run_id, chain FROM runs WHERE run_id = ANY($1)")
            .bind(&run_ids)
            .fetch_all(&state.db)
            .await?;

    // Same FROM shape as `/api/agents` — the name is needed in the response,
    // and a missing document row is precisely a NULL name, not a missing
    // agent.
    let from_sql = "FROM agent_snapshots s \
         LEFT JOIN agent_documents d \
           ON d.run_id = s.run_id AND d.chain = s.chain AND d.agent_id = s.agent_id";

    // One query for all runs, `row_number()` partitioned per run: each run
    // keeps its own top-[`ITEMS_PER_RUN`] by the shared relevance, and one
    // run with thousands of matches cannot crowd another out. The
    // (chain, agent_id) tiebreak keeps equal-relevance rows in the same
    // stable order the directory uses.
    let items_sql = format!(
        "SELECT run_id, chain, agent_id, owner, agent_uri, block_number, observed_at, name \
         FROM ( \
           SELECT s.run_id, s.chain, s.agent_id, s.owner, s.agent_uri, s.block_number, \
                  s.observed_at, d.name, \
                  row_number() OVER ( \
                    PARTITION BY s.run_id \
                    ORDER BY {relevance}, s.chain, s.agent_id \
                  ) AS rn \
           {from_sql} \
           WHERE s.run_id = ANY($1::uuid[]) AND {q_match} \
         ) ranked \
         WHERE rn <= $3 \
         ORDER BY run_id, rn",
        relevance = q_relevance_sql("$2"),
        q_match = q_match_sql("$2"),
    );
    let rows: Vec<MatchRow> = sqlx::query_as(&items_sql)
        .bind(&run_ids)
        .bind(&q)
        .bind(ITEMS_PER_RUN)
        .fetch_all(&state.db)
        .await?;

    // Totals per run, same filter — what lets the UI say "312 more on Base"
    // beyond the capped head.
    let totals_sql = format!(
        "SELECT s.run_id, count(*) {from_sql} \
         WHERE s.run_id = ANY($1::uuid[]) AND {}",
        q_match_sql("$2"),
    );
    let totals: Vec<(Uuid, i64)> = sqlx::query_as(&format!("{totals_sql} GROUP BY s.run_id"))
        .bind(&run_ids)
        .bind(&q)
        .fetch_all(&state.db)
        .await?;

    // Rung statuses for exactly the returned rows, keyed by (run_id,
    // agent_id) pairs via parallel-array unnest — `agent_id = ANY(...)` alone
    // would drag in the same agent's rows from every OTHER requested run.
    // Separate query for the same reason as in `/api/agents`: a join would
    // multiply each snapshot row per rung and break the per-run cap.
    //
    // `chain` travels with the key and is joined on, because
    // `check_results_unique` is (run_id, chain, agent_id, rung): a join given
    // `run_id` and `agent_id` but not `chain` can use only the leading column
    // and then scans each run's rows to match the rest. Every row here already
    // knows its chain, so carrying it costs one more array.
    let mut key_runs: Vec<Uuid> = Vec::with_capacity(rows.len());
    let mut key_chains: Vec<String> = Vec::with_capacity(rows.len());
    let mut key_agents: Vec<i64> = Vec::with_capacity(rows.len());
    for r in &rows {
        key_runs.push(r.run_id);
        key_chains.push(r.chain.clone());
        key_agents.push(r.agent_id);
    }
    let rung_rows: Vec<RungStatusRow> = if key_runs.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(
            "SELECT c.run_id, c.agent_id, c.rung, c.name, c.status \
             FROM check_results c \
             JOIN unnest($1::uuid[], $2::text[], $3::bigint[]) AS k(run_id, chain, agent_id) \
               ON c.run_id = k.run_id AND c.chain = k.chain AND c.agent_id = k.agent_id \
             ORDER BY c.agent_id, c.rung",
        )
        .bind(&key_runs)
        .bind(&key_chains)
        .bind(&key_agents)
        .fetch_all(&state.db)
        .await?
    };

    // Assemble in the CALLER's order, not the database's.
    let groups = run_ids
        .iter()
        .filter_map(|run_id| {
            let chain = known.iter().find(|(id, _)| id == run_id)?.1.clone();
            let total = totals
                .iter()
                .find(|(id, _)| id == run_id)
                .map(|(_, n)| *n)
                .unwrap_or(0);
            let items = rows
                .iter()
                .filter(|r| r.run_id == *run_id)
                .map(|r| AgentListItem {
                    chain: r.chain.clone(),
                    agent_id: r.agent_id,
                    owner: r.owner.clone(),
                    agent_uri: r.agent_uri.clone(),
                    block_number: r.block_number,
                    observed_at: r.observed_at,
                    name: r.name.clone(),
                    rungs: rung_rows
                        .iter()
                        .filter(|rr| rr.run_id == *run_id && rr.agent_id == r.agent_id)
                        .map(|rr| RungStatus {
                            rung: rr.rung,
                            name: rr.name.clone(),
                            status: rr.status.clone(),
                        })
                        .collect(),
                })
                .collect();
            Some(RunGroup {
                run_id: *run_id,
                chain,
                total,
                items,
            })
        })
        .collect();

    Ok(Json(groups))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn a_comma_separated_list_parses_in_order() {
        let a = uuid(1);
        let b = uuid(2);
        let parsed = parse_runs(&format!("{a},{b}")).unwrap();
        assert_eq!(parsed, vec![a, b]);
    }

    #[test]
    fn whitespace_and_empty_segments_are_tolerated_not_errors() {
        // `runs=a,,b` and `runs=a, b` are what URL-building code actually
        // emits; rejecting them would 400 on formatting, not on meaning.
        let a = uuid(1);
        let b = uuid(2);
        let parsed = parse_runs(&format!(" {a} ,, {b},")).unwrap();
        assert_eq!(parsed, vec![a, b]);
    }

    #[test]
    fn duplicates_collapse_and_the_first_occurrence_keeps_its_place() {
        let a = uuid(1);
        let b = uuid(2);
        let parsed = parse_runs(&format!("{a},{b},{a}")).unwrap();
        assert_eq!(parsed, vec![a, b]);
    }

    #[test]
    fn a_non_uuid_is_a_400_naming_the_offending_value() {
        let err = parse_runs("not-a-uuid").unwrap_err();
        match err {
            ApiError::BadRequest(msg) => assert!(msg.contains("not-a-uuid"), "got: {msg}"),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_list_is_a_400() {
        assert!(matches!(parse_runs(""), Err(ApiError::BadRequest(_))));
        assert!(matches!(parse_runs(" , ,"), Err(ApiError::BadRequest(_))));
    }

    #[test]
    fn the_run_cap_is_enforced_after_deduplication() {
        // MAX_RUNS distinct ids pass; one more fails. Dedup first, so a
        // repeated id cannot trip the cap.
        let many: Vec<String> = (0..MAX_RUNS as u128)
            .map(|n| uuid(n + 1).to_string())
            .collect();
        assert!(parse_runs(&many.join(",")).is_ok());

        let doubled = format!("{},{}", many.join(","), uuid(1));
        assert_eq!(parse_runs(&doubled).unwrap().len(), MAX_RUNS);

        let over = format!("{},{}", many.join(","), uuid(MAX_RUNS as u128 + 1));
        assert!(matches!(parse_runs(&over), Err(ApiError::BadRequest(_))));
    }

    #[test]
    fn the_shared_match_fragment_binds_only_the_placeholder_it_is_given() {
        // /api/agents binds `q` at $7, /api/search at $2 — a stray hardcoded
        // placeholder in the shared fragment would make one of them read a
        // different parameter entirely.
        let sql = q_match_sql("$2");
        assert!(!sql.contains("$7"), "got: {sql}");
        assert_eq!(sql.matches("$2").count(), 4, "got: {sql}");
        assert!(!q_relevance_sql("$2").contains("$7"));
    }
}
