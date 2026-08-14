//! Deltas — what changed between one run and the previous one, read back.
//!
//! Registration counts go up, and everyone publishes them. `stopped_resolving`
//! is the number nobody else can produce, because it requires having asked the
//! same question of the same population at two pinned blocks and kept both
//! answers. This endpoint serves that number — it does not compute it.
//!
//! ## Why a two-run figure is allowed here
//!
//! The README's rule is that every query is run-scoped, because blending rows
//! from two runs compares an agent to itself across two points in time as if
//! they were one. A delta is the one legitimate two-run comparison, and it is
//! legitimate precisely because it was made ONCE, at sweep time, by
//! `sweeper::delta::compute` over both runs' full `check_results`, and stored
//! in `run_deltas` naming both run ids. This handler reads a single stored
//! row; it never joins two runs itself, so a delta served today and the same
//! delta served next month describe the same comparison.
//!
//! ## The two rules a consumer must not be able to escape
//!
//! * **The confound travels with the number.** When `checker_before` differs
//!   from `checker_after` (or the schema versions differ), an unknown share of
//!   the flips is method, not the world — migration 0016's own words: "Any
//!   published figure must say so." All four columns are served, plus
//!   `method_changed`, so no client has to remember the comparison.
//! * **A missing delta is a 404, never zeros.** A run with no predecessor has
//!   no `run_deltas` row at all, because "first observation" and "nothing
//!   changed" are different claims and a row of zeroes reads as the second.
//!   The same absence-is-not-a-status rule as everywhere else in this API.
//!
//! Transitions into or out of `refused` are already excluded from
//! `stopped_resolving` and `newly_resolving` by `sweeper::delta::compute` —
//! by rule, not here (see the 2026-08-06 methodology changelog entry: a rate
//! limit of ours was briefly published as 19,983 agents going dark). They
//! remain in `flips`, and `rung2_declined` totals them so a reader can see
//! the excluded volume without deriving it client-side.

use axum::Json;
use axum::extract::{Path, State};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::AppState;
use crate::error::{ApiError, ApiResult};

/// One `run_deltas` row, exactly as the `delta` binary wrote it — no derived
/// fields except the two spelled out below, so nothing here can drift from
/// what was computed at sweep time.
#[derive(Debug, sqlx::FromRow)]
struct DeltaRow {
    run_id: Uuid,
    previous_run_id: Uuid,
    chain: String,
    agents_before: i32,
    agents_after: i32,
    newly_registered: i32,
    disappeared: i32,
    newly_resolving: i32,
    stopped_resolving: i32,
    flips: serde_json::Value,
    checker_before: String,
    checker_after: String,
    schema_before: i32,
    schema_after: i32,
    computed_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct DeltaResponse {
    /// The newer run — the one this delta belongs to.
    pub run_id: Uuid,
    /// What it is compared against: the previous finished run on the same
    /// chain at the time the delta was computed. Both runs are in `/api/runs`,
    /// so a consumer can name the pair's pinned blocks and dates.
    pub previous_run_id: Uuid,
    pub chain: String,
    /// Distinct agents with any check result in the older / newer run. Not
    /// necessarily equal to either run's `agent_count`.
    pub agents_before: i32,
    pub agents_after: i32,
    /// Present in the newer run, absent from the older.
    pub newly_registered: i32,
    /// Present in the older run, absent from the newer. Expected to be 0 —
    /// an ERC-721 is not usually burned — so a non-zero value is a finding.
    pub disappeared: i32,
    /// Rung 2 went from a not-pass to `pass`. A transition out of `refused`
    /// is excluded: getting through this week after being declined last week
    /// is not the agent having come back.
    pub newly_resolving: i32,
    /// Rung 2 went from `pass` to a not-pass. A transition into `refused`
    /// (429, 503, an auth/payment challenge, a robots.txt that declined us)
    /// is excluded: the origin declined us, which is not the agent having
    /// gone away.
    pub stopped_resolving: i32,
    /// Rung-2 transitions into or out of `refused` — the volume the two
    /// series above exclude, totalled so the exclusion is visible rather
    /// than silent. Derived here from `flips`, which retains every one.
    pub rung2_declined: i64,
    /// Every status transition, including the refused ones:
    /// `[{"rung", "from", "to", "agents"}, …]`, sorted by (rung, from, to).
    pub flips: serde_json::Value,
    /// When these differ, some flips are method changes rather than changes
    /// in the world. Any published figure must say so.
    pub checker_before: String,
    pub checker_after: String,
    pub schema_before: i32,
    pub schema_after: i32,
    /// True iff the checker version or schema version differs across the
    /// pair. Served precomputed so no consumer has to remember to compare.
    pub method_changed: bool,
    pub computed_at: DateTime<Utc>,
}

/// Sum of `agents` over rung-2 flips that enter or leave `refused` — the
/// volume `stopped_resolving` and `newly_resolving` exclude by rule.
fn rung2_declined(flips: &serde_json::Value) -> i64 {
    let Some(rows) = flips.as_array() else {
        return 0;
    };
    rows.iter()
        .filter(|f| f.get("rung").and_then(|v| v.as_i64()) == Some(2))
        .filter(|f| {
            let word = |key| f.get(key).and_then(|v| v.as_str());
            word("from") == Some("refused") || word("to") == Some("refused")
        })
        .filter_map(|f| f.get("agents").and_then(|v| v.as_i64()))
        .sum()
}

/// `GET /api/runs/{id}/delta` — the stored comparison for one run, or 404.
///
/// 404 covers two different absences and deliberately does not distinguish
/// them: an unknown run id, and a real run with no predecessor (the first
/// sweep of a chain). Neither has a delta, and inventing a zero-filled row
/// for the second would publish "nothing changed" about a comparison that
/// never happened.
pub async fn get(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> ApiResult<Json<DeltaResponse>> {
    let row = sqlx::query_as::<_, DeltaRow>(
        "SELECT run_id, previous_run_id, chain, agents_before, agents_after, \
                newly_registered, disappeared, newly_resolving, stopped_resolving, \
                flips, checker_before, checker_after, schema_before, schema_after, \
                computed_at \
           FROM run_deltas WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let declined = rung2_declined(&row.flips);
    Ok(Json(DeltaResponse {
        run_id: row.run_id,
        previous_run_id: row.previous_run_id,
        chain: row.chain,
        agents_before: row.agents_before,
        agents_after: row.agents_after,
        newly_registered: row.newly_registered,
        disappeared: row.disappeared,
        newly_resolving: row.newly_resolving,
        stopped_resolving: row.stopped_resolving,
        rung2_declined: declined,
        flips: row.flips,
        checker_before: row.checker_before.clone(),
        checker_after: row.checker_after.clone(),
        schema_before: row.schema_before,
        schema_after: row.schema_after,
        method_changed: row.checker_before != row.checker_after
            || row.schema_before != row.schema_after,
        computed_at: row.computed_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The exclusion the two headline series rely on: `rung2_declined` totals
    /// exactly the rung-2 transitions that touch `refused`, and nothing else.
    #[test]
    fn declined_totals_only_rung2_refused_transitions() {
        let flips = json!([
            {"rung": 2, "from": "pass", "to": "refused", "agents": 19_658},
            {"rung": 2, "from": "refused", "to": "pass", "agents": 304},
            {"rung": 2, "from": "pass", "to": "fail", "agents": 10},
            {"rung": 4, "from": "pass", "to": "refused", "agents": 7},
            {"rung": 7, "from": "fail", "to": "pass", "agents": 3},
        ]);
        assert_eq!(rung2_declined(&flips), 19_658 + 304);
    }

    #[test]
    fn declined_is_zero_for_empty_or_malformed_flips() {
        assert_eq!(rung2_declined(&json!([])), 0);
        assert_eq!(rung2_declined(&json!({})), 0);
        assert_eq!(rung2_declined(&json!(null)), 0);
    }

    /// The response always carries the confound: serializing a delta yields
    /// the four before/after method columns and the precomputed comparison,
    /// so no client can render churn without them.
    #[test]
    fn serialized_delta_carries_the_method_confound() {
        let response = DeltaResponse {
            run_id: Uuid::nil(),
            previous_run_id: Uuid::nil(),
            chain: "bsc".into(),
            agents_before: 244_208,
            agents_after: 263_181,
            newly_registered: 18_973,
            disappeared: 0,
            newly_resolving: 12,
            stopped_resolving: 10,
            rung2_declined: 19_962,
            flips: json!([]),
            checker_before: "0.6.0".into(),
            checker_after: "0.7.0".into(),
            schema_before: 7,
            schema_after: 8,
            method_changed: true,
            computed_at: DateTime::<Utc>::MIN_UTC,
        };
        let value = serde_json::to_value(&response).unwrap();
        for key in [
            "checker_before",
            "checker_after",
            "schema_before",
            "schema_after",
            "method_changed",
            "rung2_declined",
        ] {
            assert!(value.get(key).is_some(), "response lost `{key}`");
        }
        assert_eq!(value["method_changed"], json!(true));
    }
}
