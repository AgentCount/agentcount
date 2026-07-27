//! Aggregate statistics — raw counts for the research post. No fractions of
//! "fake reputation": the report derives its own claims from flags + evidence.

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::AppState;
use crate::error::ApiResult;

/// The aggregate numbers. Every field is fodder for a chart in the write-up.
#[derive(Debug, Serialize)]
pub struct Stats {
    pub total_agents: i64,
    pub live_endpoints: i64,
    pub payable_endpoints: i64,
    pub metadata_resolving: i64,
    pub flagged_agents: i64,
    /// Flag counts by kind, most-flagged first.
    pub flags_by_kind: Vec<FlagKindCount>,
}

/// One row of the flags-by-kind breakdown.
///
/// An ordered array rather than a JSON object: a chart needs a stable order,
/// and object key order is not a guarantee anyone should rely on. The `label`
/// travels with the count so a dashboard never has to turn `shared_operator`
/// into "shared operator" itself — that wording belongs to the facts crate.
#[derive(Debug, Serialize)]
pub struct FlagKindCount {
    pub kind: String,
    pub label: String,
    pub count: i64,
}

/// `GET /api/stats` — a handful of aggregate queries assembled into `Stats`.
///
/// `query_scalar` is the shortcut for a query that returns a single value.
pub async fn summary(State(state): State<AppState>) -> ApiResult<Json<Stats>> {
    let total_agents: i64 = sqlx::query_scalar("SELECT count(*) FROM agents")
        .fetch_one(&state.db)
        .await?;

    let live_endpoints: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_enrichment WHERE endpoint_healthy")
            .fetch_one(&state.db)
            .await?;

    // "Payable" = at least one 402 observed in the probe history — the x402
    // signal, and a count nobody else is publishing.
    let payable_endpoints: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT (chain, agent_id)) FROM probe_history WHERE outcome = 'payment_required'",
    )
    .fetch_one(&state.db)
    .await?;

    let metadata_resolving: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT (chain, agent_id)) FROM metadata_snapshots WHERE body IS NOT NULL",
    )
    .fetch_one(&state.db)
    .await?;

    let flagged_agents: i64 =
        sqlx::query_scalar("SELECT count(DISTINCT (chain, agent_id)) FROM flags")
            .fetch_one(&state.db)
            .await?;

    // Ordered by count so the breakdown renders the same way every request;
    // `kind` breaks ties so the order is total.
    #[derive(sqlx::FromRow)]
    struct KindRow {
        kind: String,
        count: i64,
    }
    let flags_by_kind: Vec<FlagKindCount> = sqlx::query_as::<_, KindRow>(
        "SELECT kind, count(*) AS count FROM flags GROUP BY kind \
         ORDER BY count DESC, kind",
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|r| FlagKindCount {
        label: facts::flag_label(&r.kind),
        kind: r.kind,
        count: r.count,
    })
    .collect();

    Ok(Json(Stats {
        total_agents,
        live_endpoints,
        payable_endpoints,
        metadata_resolving,
        flagged_agents,
        flags_by_kind,
    }))
}
