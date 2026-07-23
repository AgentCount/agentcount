//! Server-rendered pages: the same facts the API serves, as HTML. Both go
//! through `facts_view::assemble`, so site and API cannot disagree.
//!
//! "Server-rendered" means the HTML is assembled here and sent ready-to-display
//! — no client-side framework. Simpler, faster first paint, and keeps the
//! focus on the Rust.

use askama::Template;
use axum::extract::{Path, State};
use axum::response::Html;

use crate::error::{ApiError, ApiResult};
use crate::facts_view;
use crate::templates::{AgentDetailPage, AgentRow, ExplorerPage, FactRow, FlagRow, MethodologyPage};
use crate::AppState;

/// `GET /` — all agents, newest registration first. A directory, not a leaderboard.
pub async fn explorer(State(state): State<AppState>) -> ApiResult<Html<String>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        chain: String,
        agent_id: i64,
        domain: String,
        endpoint_alive: bool,
        registered_at: chrono::DateTime<chrono::Utc>,
        flag_count: i64,
    }
    let rows = sqlx::query_as::<_, Row>(
        "SELECT a.chain, a.agent_id, a.domain, a.registered_at, \
                COALESCE(e.endpoint_healthy, false) AS endpoint_alive, \
                COALESCE(fl.n, 0) AS flag_count \
         FROM agents a \
         LEFT JOIN agent_enrichment e ON e.chain = a.chain AND e.agent_id = a.agent_id \
         LEFT JOIN (SELECT chain, agent_id, count(*) AS n FROM flags GROUP BY chain, agent_id) fl \
                ON fl.chain = a.chain AND fl.agent_id = a.agent_id \
         ORDER BY a.registered_at DESC LIMIT 200",
    )
    .fetch_all(&state.db)
    .await?;

    let agents = rows
        .into_iter()
        .map(|r| AgentRow {
            agent_id: r.agent_id,
            chain: r.chain,
            domain: r.domain,
            is_alive: r.endpoint_alive,
            registered: r.registered_at.format("%Y-%m-%d").to_string(),
            flag_count: r.flag_count,
        })
        .collect();

    Ok(Html(ExplorerPage { agents }.render()?))
}

/// `GET /agent/{chain}/{id}` — facts and flags for one agent.
pub async fn agent_detail(
    State(state): State<AppState>,
    Path((chain, agent_id)): Path<(String, i64)>,
) -> ApiResult<Html<String>> {
    let assembled = facts_view::assemble(&state.db, &chain, agent_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    // Facts → display rows. The phrasing here mirrors the fact JSON exactly;
    // no interpretation is added between the data and the reader.
    let facts = assembled
        .facts
        .iter()
        .map(|f| {
            let v = &f.value;
            let (label, value) = match f.kind {
                "registered_since" => (
                    "Registered".to_string(),
                    format!(
                        "since {} on {}",
                        v["registered_at"].as_str().unwrap_or("?"),
                        v["chain"].as_str().unwrap_or("?")
                    ),
                ),
                "endpoint_liveness" => (
                    "Endpoint liveness".to_string(),
                    format!("answered {} of {} probes in the last 30 days", v["alive"], v["probes"]),
                ),
                "payable_endpoint" => (
                    "Payable endpoint".to_string(),
                    format!(
                        "returned HTTP 402 (payment required) on {} probes",
                        v["payment_required_responses"]
                    ),
                ),
                "metadata_status" => (
                    "Metadata".to_string(),
                    format!(
                        "{} ({} snapshots archived)",
                        v["status"].as_str().unwrap_or("?"),
                        v["snapshots_archived"]
                    ),
                ),
                "attestations" => (
                    "Attestations".to_string(),
                    format!("{} recorded on-chain, {} mutual", v["total"], v["mutual"]),
                ),
                "validation_proofs" => (
                    "Validation proofs".to_string(),
                    format!(
                        "{} ({} passed, {} failed)",
                        v["status"].as_str().unwrap_or("?"),
                        v["passed"],
                        v["failed"]
                    ),
                ),
                other => (other.to_string(), v.to_string()),
            };
            let evidence = f
                .evidence
                .iter()
                .map(|e| match e {
                    facts::EvidenceRef::Tx { chain, tx_hash } => format!("tx {tx_hash} ({chain})"),
                    facts::EvidenceRef::Snapshot { snapshot_id } => format!("snapshot #{snapshot_id}"),
                    facts::EvidenceRef::ProbeWindow { probes, .. } => format!("{probes} archived probes"),
                    facts::EvidenceRef::Registry { chain } => format!("{chain} registry events"),
                })
                .collect::<Vec<_>>()
                .join(", ");
            FactRow { label, value, evidence }
        })
        .collect();

    let flags = assembled
        .flags
        .iter()
        .map(|fl| {
            let peers = fl.evidence["peers"].as_array().map(|p| p.len()).unwrap_or(0);
            let detail = match fl.kind.as_str() {
                "shared_operator" => format!(
                    "operated by the same wallet ({}) as {} other agent(s)",
                    fl.evidence["address"].as_str().unwrap_or("?"),
                    peers
                ),
                "synchronized_registration" => {
                    format!("registered in a burst of {} agents within one window", fl.evidence["count"])
                }
                "reciprocal_feedback" => format!("mutual rating pair(s) with {} agent(s)", peers),
                other => other.to_string(),
            };
            FlagRow {
                label: fl.kind.replace('_', " "),
                detail,
                raised: fl.raised_at.format("%Y-%m-%d").to_string(),
            }
        })
        .collect();

    let page = AgentDetailPage {
        agent_id: assembled.summary.agent_id,
        chain: assembled.summary.chain,
        domain: assembled.summary.domain,
        address: assembled.summary.address,
        facts,
        flags,
    };
    Ok(Html(page.render()?))
}

/// `GET /methodology` — what we measure and how; no formulas, no weights.
pub async fn methodology() -> ApiResult<Html<String>> {
    Ok(Html(MethodologyPage {}.render()?))
}
