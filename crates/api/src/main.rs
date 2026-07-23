//! # api — serve the trust intelligence to the world
//!
//! The public face of Ledgerscope. It exposes a free JSON API and a small
//! server-rendered explorer website. It reads from Postgres, calls the
//! [`scoring`] library to turn agent data into a [`scoring::TrustScore`], and
//! renders results as JSON or HTML.
//!
//! ## Rust concepts this crate is here to teach
//!
//! * **axum handlers are just async functions.** A route handler is an
//!   `async fn` whose *arguments* declare what it needs from the request
//!   (path params, query string, shared state). axum's "extractors" make those
//!   arguments work — and quietly teach you Rust's trait system, because an
//!   extractor is any type implementing the `FromRequestParts` trait.
//! * **Shared state via `State`.** The database pool is created once and shared
//!   with every handler through `axum::extract::State`. No globals.
//! * **Errors that become HTTP responses.** Our [`error::ApiError`] implements
//!   `IntoResponse`, so a handler can just `?` on a failing query and the client
//!   gets a clean 500 instead of the process crashing.

mod error;
mod routes;
mod templates;

/// Application state shared with every request handler.
///
/// axum clones this for each request, so everything inside must be cheap to
/// clone — a `PgPool` is (it's reference-counted internally). Anything you want
/// every handler to reach (config, the db pool, caches) goes here.
#[derive(Clone)]
pub struct AppState {
    // pub db: sqlx::PgPool,
    pub db: PoolPlaceholder,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    //     tracing_subscriber::fmt().with_env_filter(
    //         tracing_subscriber::EnvFilter::from_default_env()).init();

    // 1. Connect to Postgres and build the shared state.
    //     let db = sqlx::postgres::PgPoolOptions::new()
    //         .max_connections(10)
    //         .connect(&std::env::var("DATABASE_URL")?)
    //         .await?;
    //     let state = AppState { db };

    // 2. Build the router. Each `.route(path, method(handler))` wires a URL to a
    //    handler function. `.with_state(state)` makes `AppState` available to all
    //    of them via the `State` extractor.
    //
    //     let app = axum::Router::new()
    //         // JSON API
    //         .route("/api/agents",            axum::routing::get(routes::agents::list))
    //         .route("/api/agents/{id}",       axum::routing::get(routes::agents::get_one))
    //         .route("/api/agents/{id}/score", axum::routing::get(routes::agents::get_score))
    //         .route("/api/stats",             axum::routing::get(routes::stats::summary))
    //         // Server-rendered HTML pages
    //         .route("/",             axum::routing::get(routes::pages::explorer))
    //         .route("/agent/{id}",   axum::routing::get(routes::pages::agent_detail))
    //         .route("/methodology",  axum::routing::get(routes::pages::methodology))
    //         .with_state(state);

    // 3. Bind a TCP port and serve. `axum::serve` runs until the process stops.
    //     let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    //     tracing::info!("Ledgerscope API listening on http://0.0.0.0:8080");
    //     axum::serve(listener, app).await?;
    //     Ok(())

    todo!("connect db → build the Router → bind a port → axum::serve (see the sketch)")
}

/// Placeholder for `sqlx::PgPool`. Delete once sqlx is wired in.
#[derive(Clone)]
pub struct PoolPlaceholder;
