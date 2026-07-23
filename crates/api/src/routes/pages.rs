//! The server-rendered HTML explorer.
//!
//! These handlers return *HTML* instead of JSON. Each loads data, builds an
//! askama template struct (from `templates.rs`) with everything pre-formatted,
//! and renders it to a string axum serves as `text/html`.
//!
//! "Server-rendered" means the HTML is assembled here and sent ready-to-display —
//! no client-side framework. Simpler, faster first paint, and keeps the focus on
//! the Rust.

use axum::extract::{Path, State};
use axum::response::Html;
use askama::Template;

use crate::error::{ApiError, ApiResult};
use crate::templates::{AgentDetailPage, AgentRow, ExplorerPage, MethodologyPage};
use crate::AppState;

/// Convert a `[0, 1]` score into a whole-number percentage for display.
fn pct(score: f64) -> i64 {
    (score * 100.0).round() as i64
}

/// `GET /` — the leaderboard of agents ranked by trust score.
pub async fn explorer(State(state): State<AppState>) -> ApiResult<Html<String>> {
    // Row shape from the DB; mapped into the display-ready `AgentRow` below.
    #[derive(sqlx::FromRow)]
    struct Row {
        chain: String,
        agent_id: i64,
        domain: String,
        final_score: f64,
        endpoint_healthy: bool,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT latest.chain, latest.agent_id, latest.domain, latest.final_score, \
                COALESCE(e.endpoint_healthy, false) AS endpoint_healthy \
         FROM ( \
            SELECT DISTINCT ON (s.chain, s.agent_id) \
                s.chain, s.agent_id, a.domain, s.final_score, s.computed_at \
            FROM scores s \
            JOIN agents a ON a.chain = s.chain AND a.agent_id = s.agent_id \
            ORDER BY s.chain, s.agent_id, s.computed_at DESC \
         ) latest \
         LEFT JOIN agent_enrichment e \
            ON e.chain = latest.chain AND e.agent_id = latest.agent_id \
         ORDER BY latest.final_score DESC \
         LIMIT 100",
    )
    .fetch_all(&state.db)
    .await?;

    // `map` transforms each DB row into a display row; `collect` gathers them.
    let agents = rows
        .into_iter()
        .map(|r| AgentRow {
            agent_id: r.agent_id,
            domain: r.domain,
            chain: r.chain,
            final_pct: pct(r.final_score),
            is_alive: r.endpoint_healthy,
        })
        .collect();

    // `.render()` is the method askama generated from explorer.html; `?` turns a
    // render error into a 500 via `From<askama::Error>`.
    Ok(Html(ExplorerPage { agents }.render()?))
}

/// `GET /agent/{id}` — one agent's full score breakdown and cluster status.
pub async fn agent_detail(
    State(state): State<AppState>,
    Path(agent_id): Path<i64>,
) -> ApiResult<Html<String>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        chain: String,
        agent_id: i64,
        domain: String,
        suspicion: f64,
        payment: f64,
        liveness: f64,
        age: f64,
        reputation: f64,
        sybil_penalty: f64,
        final_score: f64,
        cluster_size: i64,
    }

    // `JOIN LATERAL` lets the subquery reference the outer row (`a`), so we can
    // grab each agent's single most-recent score row inline.
    let row = sqlx::query_as::<_, Row>(
        "SELECT a.chain, a.agent_id, a.domain, a.suspicion, \
                sc.payment, sc.liveness, sc.age, sc.reputation, \
                sc.sybil_penalty, sc.final_score, \
                COALESCE(cl.cluster_size, 1) AS cluster_size \
         FROM agents a \
         JOIN LATERAL ( \
            SELECT * FROM scores s \
            WHERE s.chain = a.chain AND s.agent_id = a.agent_id \
            ORDER BY s.computed_at DESC LIMIT 1 \
         ) sc ON true \
         LEFT JOIN ( \
            SELECT cm.chain, cm.agent_id, c2.cnt AS cluster_size \
            FROM cluster_members cm \
            JOIN ( \
                SELECT cluster_id, count(*) AS cnt \
                FROM cluster_members GROUP BY cluster_id \
            ) c2 ON c2.cluster_id = cm.cluster_id \
         ) cl ON cl.chain = a.chain AND cl.agent_id = a.agent_id \
         WHERE a.agent_id = $1 LIMIT 1",
    )
    .bind(agent_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let page = AgentDetailPage {
        agent_id: row.agent_id,
        domain: row.domain,
        chain: row.chain,
        final_pct: pct(row.final_score),
        payment_pct: pct(row.payment),
        liveness_pct: pct(row.liveness),
        age_pct: pct(row.age),
        reputation_pct: pct(row.reputation),
        sybil_pct: pct(row.sybil_penalty),
        in_cluster: row.cluster_size > 1,
        cluster_size: row.cluster_size,
        suspicion_pct: pct(row.suspicion),
    };

    Ok(Html(page.render()?))
}

/// `GET /methodology` — the human-readable explanation of how scores work.
///
/// The weights come from `scoring::ScoreWeights::default()` (not hard-coded HTML),
/// so the page can never drift from the code.
pub async fn methodology() -> ApiResult<Html<String>> {
    let w = scoring::ScoreWeights::default();
    let page = MethodologyPage {
        payment_w: format!("{:.2}", w.payment),
        liveness_w: format!("{:.2}", w.liveness),
        age_w: format!("{:.2}", w.age),
        reputation_w: format!("{:.2}", w.reputation),
    };
    Ok(Html(page.render()?))
}
