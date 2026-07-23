//! # indexer — turn raw chain events into rows in Postgres
//!
//! Connects to every ENABLED chain in the `chains` table, watches its ERC-8004
//! registries, decodes events, and writes them with true on-chain provenance.
//! Which chains, which addresses, and where backfill starts are all data —
//! this binary contains no chain-specific code.
//!
//! ## Rust concepts this crate is here to teach
//!
//! * **`async` + the runtime** — `async fn`s do nothing on their own; a *runtime*
//!   drives them. `#[tokio::main]` starts that runtime and hands control to our
//!   async `main`.
//! * **Concurrency with `tokio::spawn`** — one ingest loop per chain, as
//!   separate tasks that interleave on the runtime.
//! * **`anyhow::Result` + `?`** — binaries favour one flexible error type and let
//!   `?` bubble failures up to `main`, which prints them and sets the exit code.

mod bindings;
mod chains;
mod ingest;
mod store;

use anyhow::Context;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = store::Db::connect(&database_url).await?;

    let configs = db.load_enabled_chains().await?;
    anyhow::ensure!(!configs.is_empty(), "no enabled chains — run scripts/seed_chains.sql");

    // One ingest task per enabled chain. RPC URLs come from env (secrets never
    // live in the database): RPC_URL_<CHAIN>, e.g. RPC_URL_BASE.
    let mut tasks = Vec::new();
    for config in configs {
        let var = config.rpc_env_var();
        let rpc_url = std::env::var(&var).with_context(|| format!("{var} must be set"))?;
        let chain = chains::Chain::connect(config, &rpc_url).await?;
        tasks.push(tokio::spawn(ingest::run(chain, db.clone())));
    }

    // If any loop dies, bring the process down so the supervisor restarts it —
    // a half-alive indexer silently falling behind is worse than a crash.
    for task in tasks {
        task.await??;
    }
    Ok(())
}
