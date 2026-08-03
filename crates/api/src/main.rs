//! # api — serve the conformance census to the world
//!
//! The public face of AgentCount: the JSON API and the only crate the outside
//! world talks to. It reads runs, agent snapshots, check results, and the HTTP
//! archive straight from Postgres (which `crates/sweeper` writes) and serves
//! them as JSON — no scoring, no judgment folded in along the way. The Next.js
//! app in the sibling `agentcount-web` repo is the frontend.
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
use axum::routing::{get, post};
use sqlx::postgres::PgPoolOptions;

/// Application state shared with every request handler.
///
/// axum clones this for each request, so everything inside must be cheap to
/// clone — a `PgPool` is (it's reference-counted internally).
#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::PgPool,
    /// The on-demand spot check's long-lived state: the shared `probe::Prober`
    /// (so its per-host concurrency cap and robots cache span the whole
    /// process, not one request), the lazily-connected chain clients, and the
    /// two rate limiters. Behind an `Arc` because none of it is cheap to clone
    /// and all of it must be *the same instance* for every request — a limiter
    /// cloned per request limits nothing.
    pub spot: std::sync::Arc<routes::spot_check::SpotCheckService>,
}

/// `GET /api/healthz` — proves the process is up AND can reach Postgres. This
/// is liveness for OUR service; a supervisor or uptime monitor hits it.
///
/// **Not `/healthz`.** That path is reserved on Cloud Run: Google's frontend
/// intercepts it for its own health checking and never forwards it to the
/// container. The failure is deeply confusing, because the container is
/// completely healthy — the identical image serves `/healthz` with a 200 when
/// run locally, and every other route works when deployed. What comes back is
/// Google's own 404 page, so it looks like the app is not routing, which sends
/// you looking at the router rather than at the platform.
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
    // Built once, at startup, and shared: the spot check's prober carries a
    // per-host concurrency cap and a robots.txt cache that only mean anything
    // if every request goes through the same instance, and its rate limiters
    // are the same story with sharper consequences. Fails the process on a bad
    // configuration rather than starting an API whose spot check would 500 —
    // the only way this constructor fails is an empty IPFS gateway list.
    let spot = std::sync::Arc::new(
        routes::spot_check::SpotCheckService::new().context("building the spot-check service")?,
    );
    let state = AppState { db, spot };

    // 2. Build the router. Each `.route(path, get(handler))` wires a URL to a
    //    handler; `.with_state(state)` makes `AppState` reachable from all of
    //    them via the `State` extractor. Note axum 0.8's `{param}` path syntax.
    let app = Router::new()
        // JSON API — chain is part of every agent identity path, and every
        // endpoint below is scoped to one run (explicit `?run=`, or the
        // latest completed one), except `/api/search`, which spans several
        // runs precisely by keeping them in separate groups.
        .route("/api/runs", get(routes::runs::list))
        .route("/api/runs/{id}/rates", get(routes::rates::get))
        .route("/api/runs/{id}/findings", get(routes::findings::get))
        .route("/api/agents", get(routes::agents::list))
        .route("/api/agents/{chain}/{id}", get(routes::agents::get_one))
        // The one read endpoint that spans runs — the caller names them (the
        // canonical set lives in the web repo, not here), and results stay
        // grouped per run. See `routes::search`.
        .route("/api/search", get(routes::search::get))
        // The two endpoints that serve rows belonging to NO run: agents the
        // chain has that no census has checked yet. Separate paths, and a
        // response shape that shares almost nothing with a census result, so
        // an unchecked agent can be found without ever being counted as a
        // measured one. See `routes::tail`.
        .route("/api/tail", get(routes::tail::list))
        .route("/api/tail/summary", get(routes::tail::summary))
        .route("/api/methodology", get(routes::methodology::get))
        // The one endpoint that writes nothing and reads no run: it judges a
        // document the caller supplies, with the same checker the sweep uses.
        .route("/api/validate", post(routes::validate::post))
        // The one endpoint that writes a row about a person. It is called by
        // the front end's own route handler, server-side, never by a browser
        // directly — which is why the browser is never told this API's
        // address. See `routes::subscribe`.
        .route("/api/subscribe", post(routes::subscribe::post))
        .route("/api/healthz", get(healthz))
        // Crude but effective public-endpoint hardening: cap request time and
        // total in-flight requests. Per-IP rate limiting is a fast-follow —
        // except on the spot check below, which has its own, because it is the
        // one endpoint that can point traffic at somebody else's server.
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(10),
        ))
        // ── Added AFTER the 10s layer, deliberately ──────────────────────────
        //
        // `Router::layer` wraps the routes registered *so far*, so a route
        // added below it is not covered by it. That is what this needs: 10
        // seconds is right for a database read and wrong for a spot check,
        // which pins a block, reads two contracts, fetches `robots.txt` and
        // then fetches a stranger's document — each with the sweeper's own
        // 5s connect / 10s total budget, unchanged so that a spot check and a
        // census row observe under identical conditions rather than the spot
        // check calling a slow host dead sooner.
        //
        // POST, not GET: a spot check sends real traffic to a third party, so
        // it must not be reachable by prefetchers, link unfurlers, crawlers or
        // an `<img src>`. See `routes::spot_check`'s module doc.
        .route(
            "/api/agents/{chain}/{id}/spot-check",
            post(routes::spot_check::post),
        )
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(45),
        ))
        .layer(tower::limit::ConcurrencyLimitLayer::new(256))
        .with_state(state);

    // 3. Bind a TCP port and serve until the process stops.
    //
    // The port comes from `$PORT` when set, defaulting to 8080. Container
    // platforms (Cloud Run among them) choose the port and inject it, and kill
    // any container that does not listen on exactly that — a hardcoded 8080 is
    // the single most common way a deploy fails there, and it fails as a
    // startup timeout rather than as anything that names the port.
    //
    // The host stays 0.0.0.0: binding 127.0.0.1 inside a container accepts
    // only connections from within the container itself, which looks identical
    // to a healthy service from the inside and unreachable from the outside.
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8080);
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!("AgentCount API listening on http://{addr}");
    // `into_make_service_with_connect_info` is what puts the TCP peer address
    // in reach of a handler. Only `routes::spot_check` uses it, as the fallback
    // key for its per-client rate limit when no proxy header is present — and
    // without this the extractor would fail at runtime rather than at compile
    // time, so the two must be changed together.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .context("serving")?;
    Ok(())
}
