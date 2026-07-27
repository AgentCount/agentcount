//! Server-rendered pages: the same facts the API serves, as HTML. Both go
//! through `facts_view::assemble`, so site and API cannot disagree.
//!
//! "Server-rendered" means the HTML is assembled here and sent ready-to-display
//! — no client-side framework. Simpler, faster first paint, and keeps the
//! focus on the Rust.

use askama::Template;
use axum::extract::{Path, State};
use axum::response::Html;

use crate::AppState;
use crate::error::{ApiError, ApiResult};
use crate::facts_view;
use crate::templates::{
    AgentDetailPage, AgentRow, ExplorerPage, FactRow, FlagRow, MethodologyPage,
};

/// `GET /` — all agents, newest registration first. A directory, not a leaderboard.
pub async fn explorer(State(state): State<AppState>) -> ApiResult<Html<String>> {
    // Same query the JSON route uses. The page keeps its historical 200-row
    // window; paging lives in the JSON API (and, from Phase B, the new UI).
    let page = facts_view::list_agents(
        &state.db,
        &facts_view::ListFilter {
            chain: None,
            limit: 200,
            offset: 0,
            sort: facts_view::Sort::Registered,
        },
    )
    .await?;

    let agents = page
        .items
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

    // Display strings come from the facts crate — this handler decides layout,
    // never wording. Dates are formatted here because a date format is not a
    // claim.
    let facts = assembled
        .facts
        .iter()
        .map(|pf| FactRow {
            label: pf.display.label.clone(),
            value: pf.display.statement.clone(),
            evidence: pf.display.evidence_summary.clone(),
        })
        .collect();

    let flags = assembled
        .flags
        .iter()
        .map(|fl| FlagRow {
            label: fl.display.label.clone(),
            detail: fl.display.statement.clone(),
            raised: fl.raised_at.format("%Y-%m-%d").to_string(),
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
