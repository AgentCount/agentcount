//! # indexer — turn raw chain events into rows in Postgres
//!
//! This binary is the mouth of the pipeline. It connects to Ethereum and Base,
//! watches the three ERC-8004 registry contracts, decodes every event they emit,
//! and writes it to the database. Everything downstream reads what this produces.
//!
//! ## Rust concepts this crate is here to teach
//!
//! * **`async` + the runtime** — `async fn`s do nothing on their own; a *runtime*
//!   drives them. `#[tokio::main]` starts that runtime and hands control to our
//!   async `main`.
//! * **Concurrency with `tokio::spawn`** — we run one ingest loop per chain as
//!   separate tasks that interleave on the runtime. Lots of mostly-waiting-on-
//!   the-network work sharing a few threads is exactly what async is best at.
//! * **`anyhow::Result` + `?`** — binaries favour one flexible error type and let
//!   `?` bubble failures up to `main`, which prints them and sets the exit code.

mod bindings;
mod chains;
mod ingest;
mod store;

use anyhow::Context;

/// Configuration read from the environment at startup.
struct Config {
    database_url: String,
    ethereum_rpc_url: String,
    base_rpc_url: String,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            database_url: std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?,
            ethereum_rpc_url: std::env::var("ETHEREUM_RPC_URL")
                .context("ETHEREUM_RPC_URL must be set")?,
            base_rpc_url: std::env::var("BASE_RPC_URL").context("BASE_RPC_URL must be set")?,
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::from_env()?;

    // Open one shared pool; each chain loop gets a cheap clone.
    let db = store::Db::connect(&config.database_url).await?;

    // Connect both chains.
    let ethereum = chains::Chain::connect("ethereum", &config.ethereum_rpc_url).await?;
    let base = chains::Chain::connect("base", &config.base_rpc_url).await?;

    // Run both ingest loops concurrently. `tokio::spawn` hands each future to the
    // runtime as an independent task; `try_join!` waits for both and short-
    // circuits if either fails. The double `?` unwraps first the JoinError (did
    // the task panic?) then the task's own `Result`.
    let eth_task = tokio::spawn(ingest::run(ethereum, db.clone()));
    let base_task = tokio::spawn(ingest::run(base, db.clone()));

    let (eth_res, base_res) = tokio::try_join!(eth_task, base_task)?;
    eth_res?;
    base_res?;

    Ok(())
}
