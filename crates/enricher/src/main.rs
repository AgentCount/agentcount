//! # enricher — add observed reality to on-chain agents
//!
//! The indexer tells us an agent *exists*. The enricher records what it *does*:
//! one guarded fetch per agent yields the liveness outcome (an HTTP 402 counts
//! as alive — that's the x402 "payable" signal) AND an archived metadata
//! snapshot, then coordination flags are raised with concrete evidence.
//!
//! One "pass" does three things, then sleeps and repeats: (1) load agents due
//! for observation, (2) observe each endpoint once with bounded concurrency
//! and append the results to history, (3) detect coordination flags across
//! the whole agent set and persist them append-only.
//!
//! ## Rust concepts this crate is here to teach
//!
//! * **Bounded concurrency** — observing thousands of endpoints one-at-a-time
//!   is slow, but all-at-once would hammer the network and get you rate-limited.
//!   `buffer_unordered(n)` keeps at most `n` observations in flight at once.
//! * **Failures as data, not errors** — a dead endpoint isn't an `Err` to
//!   bubble; it's an outcome we record. `observe()` is infallible by design.
//! * **Pure core, async shell** — flag heuristics live in a pure function
//!   (`flags::detect_flags`), unit-tested without a database.

mod flags;
mod metadata;
mod netguard;
mod observe;
mod store;

use std::time::Duration;

use anyhow::Context;
use futures::stream::{self, StreamExt};

/// Configuration read from the environment at startup.
struct Config {
    /// Postgres connection string.
    database_url: String,
    /// How many endpoints to probe concurrently. A knob you'll tune.
    probe_concurrency: usize,
    /// How long to sleep between passes.
    pass_interval: Duration,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            database_url: std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?,
            // `.ok()` turns the `Result` from `var` into an `Option` (ignoring
            // "not set"); `and_then(parse)` tries to parse it; `unwrap_or` supplies
            // the default. A common "optional env var with a default" chain.
            probe_concurrency: std::env::var("PROBE_CONCURRENCY")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(32),
            pass_interval: Duration::from_secs(
                std::env::var("ENRICH_INTERVAL_SECS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(300),
            ),
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::from_env()?;
    let db = store::Db::connect(&config.database_url).await?;

    tracing::info!("enricher started; pass interval {:?}", config.pass_interval);

    // The enricher runs as periodic passes. `loop { ... }` runs forever; each
    // iteration is one full pass, and we nap between them.
    loop {
        if let Err(e) = run_pass(&db, config.probe_concurrency).await {
            // A failed pass shouldn't kill the daemon — log it and try again next
            // time. We only `?`-abort on truly unrecoverable setup errors above.
            tracing::error!("enrichment pass failed: {e:#}");
        }
        tokio::time::sleep(config.pass_interval).await;
    }
}

/// Run exactly one enrichment pass.
async fn run_pass(db: &store::Db, concurrency: usize) -> anyhow::Result<()> {
    // 1. Which agents need refreshing?
    let agents = db.load_agents_to_enrich().await?;
    tracing::info!("enriching {} agents", agents.len());

    // 2. One observation per agent — a single guarded fetch yields liveness
    //    AND the metadata snapshot.
    //
    //    `stream::iter(agents)` turns the Vec into a stream; `.map(...)` starts an
    //    async job per agent; `.buffer_unordered(concurrency)` runs up to
    //    `concurrency` of them at once and yields results as they finish (order
    //    not preserved — we don't care). `.collect()` gathers them all.
    //
    //    One shared client for the whole pass: reqwest's Client is an Arc'd
    //    connection pool, so each job borrows it instead of building its own.
    let client = observe::build_client()?;
    let observations: Vec<observe::Observation> = stream::iter(agents)
        .map(|agent| {
            let client = &client;
            async move { observe::observe(client, &agent).await }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    // 3. Append history + refresh the cache (a failed fetch is stored as data).
    db.write_observations(&observations).await?;

    // 4. Detect coordination flags across the whole agent set (needs the
    //    global picture) and persist them, append-only at the event level.
    let flags = flags::detect(db).await?;
    tracing::info!("detected {} flags", flags.len());
    db.upsert_flags(&flags).await?;

    Ok(())
}
