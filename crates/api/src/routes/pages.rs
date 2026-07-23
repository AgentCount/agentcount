//! The server-rendered HTML explorer.
//!
//! These handlers return *HTML* instead of JSON. Each builds an askama template
//! struct (from `templates.rs`), fills it with data from the database, and
//! renders it to a string that axum serves as `text/html`.
//!
//! "Server-rendered" means the HTML is assembled here, on the server, and sent
//! ready-to-display — no React, no client-side rendering. For an explorer this
//! is simpler, faster to first paint, and keeps the focus on the Rust. That was
//! a deliberate project choice: don't disappear into a JS frontend.

use axum::extract::{Path, State};
use axum::response::Html;

use crate::error::{ApiError, ApiResult};
use crate::AppState;

/// `GET /` — the leaderboard of agents ranked by trust score.
///
/// Returns `Html<String>`: axum sets the `text/html` content-type for us. The
/// helper `render` at the bottom turns a template into that `Html<String>` (or an
/// `ApiError` if rendering fails).
pub async fn explorer(State(state): State<AppState>) -> ApiResult<Html<String>> {
    // Sketch:
    //   * Query the top N agents by latest final_score into `Vec<AgentRow>`.
    //   * let page = templates::ExplorerPage { agents };
    //   * render(page)
    let _ = state;
    todo!("load the leaderboard rows, build ExplorerPage, render it")
}

/// `GET /agent/{id}` — one agent's full score breakdown and enrichment.
pub async fn agent_detail(
    State(state): State<AppState>,
    Path(agent_id): Path<u64>,
) -> ApiResult<Html<String>> {
    // Sketch:
    //   * Assemble a `scoring::AgentView`, call `scoring::score(&view)?`.
    //   * Build `templates::AgentDetailPage { .., score, .. }` and render.
    //   * No such agent → ApiError::NotFound.
    let _ = (state, agent_id);
    todo!("build AgentDetailPage (with a real TrustScore) and render it")
}

/// `GET /methodology` — the human-readable explanation of how scores work.
///
/// Passing the live default weights (rather than hard-coding them in the HTML)
/// keeps the published methodology honest: the page can't drift from the code.
pub async fn methodology() -> ApiResult<Html<String>> {
    // let page = templates::MethodologyPage { weights: scoring::ScoreWeights::default() };
    // render(page)
    todo!("render MethodologyPage using scoring::ScoreWeights::default()")
}

/// Shared helper: render any askama template to `Html<String>`, mapping a render
/// failure to a 500. Factoring this out means every page handler is a one-liner.
///
/// The `T: askama::Template` bound is a **generic with a trait bound**: this
/// function accepts *any* type that implements the `Template` trait — one helper
/// for all three pages. Generics + trait bounds are how Rust does polymorphism
/// without inheritance.
#[allow(dead_code)]
fn render<T /*: askama::Template */>(_template: T) -> ApiResult<Html<String>> {
    // With askama wired in:
    //     template.render()
    //         .map(Html)
    //         .map_err(|e| ApiError::Internal(e.to_string()))
    let _ = ApiError::NotFound; // keep the import referenced until filled in
    todo!("call template.render(), wrap Ok in Html, map render errors to Internal")
}
