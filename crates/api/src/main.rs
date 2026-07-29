//! # api — serve the conformance census to the world
//!
//! The public face of Ledgerscope: the JSON API and the only crate the outside
//! world talks to. It reads runs, agent snapshots, check results, and the HTTP
//! archive straight from Postgres (which `crates/sweeper` writes) and serves
//! them as JSON — no scoring, no judgment folded in along the way. The Next.js
//! app in the sibling `ledgerscope-web` repo is the frontend.
//!
//! This is a rewrite, not a patch: the previous version of this crate served a
//! retired availability model whose tables (`probe_history`,
//! `metadata_snapshots`, `flags`, `agent_enrichment`) were dropped in migration
//! 0008, so every one of its endpoints had been returning 500s. Nothing here
//! reads those tables or the pure library crate that used to word them into
//! prose — that crate is deleted along with this crate's own module that
//! called into it.
//!
//! ## Rust concepts this crate is here to teach
//!
//! * **axum handlers are just async functions.** A handler's *arguments* declare
//!   what it needs from the request (shared state, a path param, a query string);
//!   axum's "extractors" supply them, quietly teaching you the trait system.
//! * **Shared state via `State`.** The database pool is created once and shared
//!   with every handler. No globals.
//! * **Errors that become HTTP responses.** [`error::ApiError`] implements
//!   `IntoResponse`, so a handler can `?` on a failing query and the client gets
//!   a clean 500 instead of a crashed process.

mod error;
mod routes;

use anyhow::Context;
use axum::Router;
use axum::routing::get;
use sqlx::postgres::PgPoolOptions;

/// Application state shared with every request handler.
///
/// axum clones this for each request, so everything inside must be cheap to
/// clone — a `PgPool` is (it's reference-counted internally).
#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::PgPool,
}

/// `GET /healthz` — proves the process is up AND can reach Postgres. This is
/// liveness for OUR service; a supervisor or uptime monitor hits it.
async fn healthz(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<&'static str, error::ApiError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.db)
        .await?;
    Ok("ok")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // 1. Connect to Postgres and build the shared state.
    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?)
        .await
        .context("connecting to Postgres")?;
    let state = AppState { db };

    // 2. Build the router. Each `.route(path, get(handler))` wires a URL to a
    //    handler; `.with_state(state)` makes `AppState` reachable from all of
    //    them via the `State` extractor. Note axum 0.8's `{param}` path syntax.
    let app = Router::new()
        // JSON API — chain is part of every agent identity path, and every
        // endpoint below is scoped to one run (explicit `?run=`, or the
        // latest completed one).
        .route("/api/runs", get(routes::runs::list))
        .route("/api/runs/{id}/rates", get(routes::rates::get))
        .route("/api/runs/{id}/findings", get(routes::findings::get))
        .route("/api/agents", get(routes::agents::list))
        .route("/api/agents/{chain}/{id}", get(routes::agents::get_one))
        .route("/api/methodology", get(routes::methodology::get))
        .route("/healthz", get(healthz))
        // Crude but effective public-endpoint hardening: cap request time and
        // total in-flight requests. Per-IP rate limiting is a fast-follow.
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(10),
        ))
        .layer(tower::limit::ConcurrencyLimitLayer::new(256))
        .with_state(state);

    // 3. Bind a TCP port and serve until the process stops.
    let addr = "0.0.0.0:8080";
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!("Ledgerscope API listening on http://{addr}");
    axum::serve(listener, app).await.context("serving")?;
    Ok(())
}
