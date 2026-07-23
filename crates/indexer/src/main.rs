//! # indexer — turn raw chain events into rows in Postgres
//!
//! This binary is the mouth of the pipeline. It connects to Ethereum and Base,
//! watches the three ERC-8004 registry contracts (Identity, Reputation,
//! Validation), decodes every event they emit, and writes it to the database.
//! Everything downstream (enricher, scoring, api) reads what this produces.
//!
//! ## Rust concepts this crate is here to teach
//!
//! * **`async` + the runtime** — Rust's `async fn`s do nothing on their own;
//!   they describe work that a *runtime* has to drive. `#[tokio::main]` below is
//!   the macro that starts that runtime and hands control to our async `main`.
//! * **`.await`** — the point where an async function politely yields control
//!   while waiting (for the network, the disk, a timer) so other tasks can run
//!   on the same thread. This is Rust's big conceptual leap; you'll see it a lot.
//! * **`anyhow::Result` + `?`** — binaries favour one flexible error type and
//!   let `?` bubble failures up to `main`, which prints them and sets the exit
//!   code. No `try/catch`, no exceptions.

mod bindings;
mod chains;
mod ingest;
mod store;

use anyhow::Context;

/// Configuration this binary reads from the environment at startup.
///
/// Keeping config in one struct (rather than calling `std::env::var` all over
/// the place) makes it obvious exactly what the program needs to run.
struct Config {
    /// Postgres connection string, e.g. `postgres://user:pass@host/db`.
    database_url: String,
    /// JSON-RPC endpoint for Ethereum mainnet.
    ethereum_rpc_url: String,
    /// JSON-RPC endpoint for Base.
    base_rpc_url: String,
}

impl Config {
    /// Read configuration from environment variables, failing with a helpful
    /// message if a required one is missing.
    fn from_env() -> anyhow::Result<Self> {
        // `std::env::var` returns `Result<String, _>`; `.context(...)` (from
        // anyhow) attaches a human-friendly explanation before `?` bubbles it up.
        //
        //     Ok(Self {
        //         database_url: std::env::var("DATABASE_URL")
        //             .context("DATABASE_URL must be set")?,
        //         ethereum_rpc_url: std::env::var("ETHEREUM_RPC_URL")
        //             .context("ETHEREUM_RPC_URL must be set")?,
        //         base_rpc_url: std::env::var("BASE_RPC_URL")
        //             .context("BASE_RPC_URL must be set")?,
        //     })
        let _ = Context::context::<(), ()>; // keep the import referenced
        todo!("read DATABASE_URL / ETHEREUM_RPC_URL / BASE_RPC_URL from the env")
    }
}

/// The `#[tokio::main]` macro rewrites this async `main` into a normal `main`
/// that boots the tokio runtime and blocks on our future. Returning
/// `anyhow::Result<()>` means any `?` that fails will print a nice error chain
/// and exit non-zero — a clean way to fail a background service.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Turn on structured logging. `RUST_LOG=indexer=info cargo run -p indexer`
    // then controls verbosity at runtime.
    //     tracing_subscriber::fmt().with_env_filter(
    //         tracing_subscriber::EnvFilter::from_default_env()).init();

    // 1. Load configuration.
    //     let config = Config::from_env()?;

    // 2. Open a shared Postgres connection pool once, and reuse it everywhere.
    //     let db = store::connect(&config.database_url).await?;

    // 3. Build a chain client per network.
    //     let ethereum = chains::Chain::connect("ethereum", &config.ethereum_rpc_url).await?;
    //     let base     = chains::Chain::connect("base", &config.base_rpc_url).await?;

    // 4. Run one ingest loop per chain, CONCURRENTLY.
    //
    //    `tokio::spawn` hands a future to the runtime to make progress on in the
    //    background, returning a handle. Two chains means two independent loops
    //    that interleave on the same runtime — this is the payoff of async: lots
    //    of mostly-waiting-on-the-network work sharing a handful of threads.
    //
    //     let eth_task  = tokio::spawn(ingest::run(ethereum, db.clone()));
    //     let base_task = tokio::spawn(ingest::run(base, db.clone()));
    //
    //    `db.clone()` is cheap: a sqlx pool is an `Arc` (atomic reference count)
    //    under the hood, so cloning bumps a counter and shares the same pool.
    //
    //    Wait for both. `tokio::try_join!` runs them together and short-circuits
    //    if either returns an error:
    //     tokio::try_join!(async { eth_task.await? }, async { base_task.await? })?;

    todo!("wire up config → db → per-chain ingest loops (see the numbered sketch)")
}
