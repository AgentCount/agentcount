//! The census's headline findings, as numerator and denominator.
//!
//! These are the numbers the homepage leads with. Every one is a population
//! count over `check_results` for a single run — the same kind of aggregate
//! `/api/runs/{id}/rates` already publishes, just cross-cut differently. There
//! is still no score anywhere: a finding is "how many agents landed in this
//! state, out of how many were asked", never a quality number for any one
//! agent, and never a tally of rungs passed.
//!
//! **Why the API and not the frontend.** The frontend renders these; it must
//! not derive them. A percentage computed in TypeScript from two numbers the
//! API happened to expose is a second implementation of the census's own
//! arithmetic, free to drift from the report. Numerator, denominator AND the
//! computed percent all come from here, so the page has nothing left to
//! decide.
//!
//! **Why no interpretation.** Each finding carries a stable `key` and the
//! populations behind it, not a sentence. The prose framing a number is
//! editorial and lives in the page; the arithmetic lives here. `label`
//! describes the DENOMINATOR — which population was asked — because a rate
//! without its denominator is the single easiest way to mislead with a true
//! number.
//!
//! Nothing in this module reads or reimplements check logic: `services_status`
//! is read from rung 4's own stored evidence, written by `crates/checks`, and
//! is never recomputed from a document body here.

use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use uuid::Uuid;

use crate::AppState;
use crate::error::{ApiError, ApiResult};

#[derive(Debug, Serialize)]
pub struct Finding {
    /// Stable identifier the frontend keys its copy off. Renaming one is a
    /// breaking change to the homepage.
    pub key: &'static str,
    pub numerator: i64,
    pub denominator: i64,
    /// Computed here so the page formats rather than derives. `None` when the
    /// denominator is zero — a rate over nobody is undefined, not 0%.
    pub percent: Option<f64>,
    /// What the denominator IS. Not a description of the finding.
    pub denominator_label: &'static str,
}

#[derive(Debug, Serialize)]
pub struct FindingsResponse {
    pub run_id: Uuid,
    pub findings: Vec<Finding>,
}

fn finding(
    key: &'static str,
    numerator: i64,
    denominator: i64,
    denominator_label: &'static str,
) -> Finding {
    Finding {
        key,
        numerator,
        denominator,
        percent: (denominator > 0).then(|| (numerator as f64) * 100.0 / (denominator as f64)),
        denominator_label,
    }
}

/// `GET /api/runs/{id}/findings`
pub async fn get(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> ApiResult<Json<FindingsResponse>> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs WHERE run_id = $1)")
        .bind(run_id)
        .fetch_one(&state.db)
        .await?;
    if !exists {
        return Err(ApiError::NotFound);
    }

    let agent_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_snapshots WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&state.db)
            .await?;

    // ── 1. Documents declaring no way to reach the agent ────────────────────
    // `services_status` is rung 4's own evidence field: 'absent' (no
    // services/endpoints key at all), 'empty' (present, zero entries), or
    // 'present'. Read, not recomputed. The denominator is every document that
    // REACHED rung 4 — i.e. parsed — which is why `status <> 'skipped'` rather
    // than `status = 'pass'`: a document that failed rung 4's one conditional
    // MUST still declared services or didn't.
    let (services_missing, rung4_reached): (i64, i64) = sqlx::query_as(
        "SELECT \
           count(*) FILTER (WHERE evidence->>'services_status' IN ('absent','empty')), \
           count(*) \
         FROM check_results \
         WHERE run_id = $1 AND rung = 4 AND status <> 'skipped'",
    )
    .bind(run_id)
    .fetch_one(&state.db)
    .await?;

    // ── 2. Conforming documents that never say which agent they belong to ───
    // Rung 5 `unclaimed` is its own status (migration 0011): the document made
    // no binding claim at all. Denominator is documents that PASSED rung 4,
    // since that is the population the claim is about.
    let unclaimed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM check_results \
         WHERE run_id = $1 AND rung = 5 AND status = 'unclaimed'",
    )
    .bind(run_id)
    .fetch_one(&state.db)
    .await?;
    let rung4_pass: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM check_results \
         WHERE run_id = $1 AND rung = 4 AND status = 'pass'",
    )
    .bind(run_id)
    .fetch_one(&state.db)
    .await?;

    // ── 3. On-chain feedback, and how it relates to having a working document
    // Rung 7 runs for every agent that passes rung 1, so its denominator is the
    // whole population. The two resolve rates underneath it are the honest
    // comparison: attested agents are, if anything, LESS likely to have a
    // document that resolves.
    let attested: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM check_results \
         WHERE run_id = $1 AND rung = 7 AND status = 'pass'",
    )
    .bind(run_id)
    .fetch_one(&state.db)
    .await?;

    // One cross-tab, four numbers: rung 7 pass/not × rung 2 pass/not.
    //
    // Written as a single grouped pass rather than a self-join. The self-join
    // this replaced was fine while one run existed and became pathological as
    // runs accumulated: Postgres used only `run_id` from
    // `idx_check_results_lookup` for the inner side and pushed `chain` and
    // `agent_id` into a join filter, so it compared every rung-7 row against
    // every rung-2 row in the run — 47.5 million rows discarded, 37 seconds,
    // past the API's own 10-second timeout. Grouping instead touches each of
    // the run's rung-2 and rung-7 rows exactly once.
    //
    // `max(status) FILTER (WHERE rung = n)` is "that row's status": the
    // `check_results_unique` constraint on (run_id, chain, agent_id, rung)
    // guarantees at most one row per rung per agent, so there is nothing for
    // `max` to choose between. Agents missing either rung are excluded, which
    // is what the inner join did.
    let (att_total, att_resolvable, unatt_total, unatt_resolvable): (i64, i64, i64, i64) =
        sqlx::query_as(
            "WITH per_agent AS ( \
               SELECT max(status) FILTER (WHERE rung = 7) AS r7, \
                      max(status) FILTER (WHERE rung = 2) AS r2 \
               FROM check_results \
               WHERE run_id = $1 AND rung IN (2, 7) \
               GROUP BY chain, agent_id \
             ) \
             SELECT \
               count(*) FILTER (WHERE r7 = 'pass'), \
               count(*) FILTER (WHERE r7 = 'pass' AND r2 = 'pass'), \
               count(*) FILTER (WHERE r7 <> 'pass'), \
               count(*) FILTER (WHERE r7 <> 'pass' AND r2 = 'pass') \
             FROM per_agent \
             WHERE r7 IS NOT NULL AND r2 IS NOT NULL",
        )
        .bind(run_id)
        .fetch_one(&state.db)
        .await?;

    Ok(Json(FindingsResponse {
        run_id,
        findings: vec![
            finding(
                "services_absent_or_empty",
                services_missing,
                rung4_reached,
                "documents that parsed and reached rung 4",
            ),
            finding(
                "registration_unclaimed",
                unclaimed,
                rung4_pass,
                "documents that passed rung 4 (conformant)",
            ),
            finding("attested", attested, agent_count, "agents in this run"),
            finding(
                "attested_resolvable",
                att_resolvable,
                att_total,
                "agents with on-chain feedback",
            ),
            finding(
                "unattested_resolvable",
                unatt_resolvable,
                unatt_total,
                "agents without on-chain feedback",
            ),
        ],
    }))
}
