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
//!
//! **Why this reads a table instead of counting.** Every finding is an
//! aggregate over one run's `check_results`, and two of them cannot be answered
//! from an index — one reads `evidence`, the other groups by `(chain,
//! agent_id)` while filtering on `rung`. Both end in a heap scan across every
//! page the run occupies, because the sweeper writes an agent's seven rungs
//! together. For the 2026-08 BNB Chain run (251,782 agents) that was about
//! 1 GB of heap reads per request and roughly 550 seconds on the production
//! instance — an HTTP 408, every time, which took the homepage's all-chains
//! figure down with it.
//!
//! So the arithmetic moved to `ls_run_findings()` (migration 0021), is run once
//! per run by the `findings` binary after a sweep, and is read back from
//! `run_findings` here. Nothing about any finding's VALUE changed; the function
//! is the endpoint's own SQL, transcribed. See migration 0021 for the plans.
//!
//! A run with no stored row still gets an answer: this module calls the same
//! function inline. That is the old cost, so the old timeout is possible again
//! on a large run — but only in the window between a sweep closing and
//! `findings` running, and it is what keeps a locally imported run
//! (`import-run`, see the README) serving the same numbers as production
//! without a second implementation to keep in step.

use std::collections::HashMap;

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

/// What the census publishes, in the order it publishes it, and what each
/// denominator IS.
///
/// This list — not `run_findings` — decides which findings exist. A key stored
/// in the table and missing here is not published; a key here and missing from
/// the table sends the whole run down the recompute path below, so a finding
/// added in code can never be served for some runs and silently omitted for
/// others.
///
/// `label` describes the DENOMINATOR, and each one's reasoning is in the
/// comments of `ls_run_findings()` and in this module's history: rung 4's
/// denominator is "reached rung 4" because a document that failed its one
/// conditional still either declared services or did not; rung 5's is
/// "passed rung 4" because that is the population the claim is about; rung 7's
/// is the whole run because rung 7 runs for every agent that passes rung 1.
const PUBLISHED: &[(&str, &str)] = &[
    (
        "services_absent_or_empty",
        "documents that parsed and reached rung 4",
    ),
    (
        "registration_unclaimed",
        "documents that passed rung 4 (conformant)",
    ),
    ("attested", "agents in this run"),
    ("attested_resolvable", "agents with on-chain feedback"),
    ("unattested_resolvable", "agents without on-chain feedback"),
];

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

    // The stored figures, written once by the `findings` binary when the sweep
    // closed. One index scan of at most a handful of rows, whatever the size of
    // the run.
    let mut counts: HashMap<String, (i64, i64)> = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT finding_key, numerator, denominator FROM run_findings WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|(k, n, d)| (k, (n, d)))
    .collect();

    // A run whose findings were never computed — imported locally, swept before
    // this table existed, or swept since a finding was added to `PUBLISHED` —
    // is counted now, from the same function. All or nothing: a partial stored
    // set must not be completed key-by-key, because the recomputed keys would
    // then be a claim about a different moment than the stored ones.
    if PUBLISHED.iter().any(|(key, _)| !counts.contains_key(*key)) {
        counts = sqlx::query_as::<_, (String, i64, i64)>(
            "SELECT finding_key, numerator, denominator FROM ls_run_findings($1)",
        )
        .bind(run_id)
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .map(|(k, n, d)| (k, (n, d)))
        .collect();
    }

    // A key the function itself did not return would be a bug in one of the two
    // — surfaced as a 500 rather than published as 0/0, which renders as an
    // honest-looking "0%" over nobody.
    let findings = PUBLISHED
        .iter()
        .map(|(key, label)| {
            let (numerator, denominator) = counts
                .get(*key)
                .copied()
                .ok_or_else(|| ApiError::Internal(format!("finding `{key}` was not computed")))?;
            Ok(finding(key, numerator, denominator, label))
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(Json(FindingsResponse { run_id, findings }))
}
