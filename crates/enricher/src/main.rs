//! # enricher — add off-chain reality to on-chain agents
//!
//! The indexer tells us an agent *exists*. The enricher tells us whether it's
//! *real*: does its endpoint respond, does it serve a valid agent-card, and —
//! most importantly — does it look like one honest actor or one node in a farm
//! of coordinated sock-puppets? As the final step of each pass it also runs the
//! pure [`scoring`] library over every agent and stores the results.
//!
//! One "pass" does four things, then sleeps and repeats: (1) load agents due for
//! enrichment, (2) probe endpoints + fetch metadata with bounded concurrency,
//! (3) detect Sybil clusters across the whole agent graph, and (4) score every
//! agent and persist the scores.
//!
//! ## Rust concepts this crate is here to teach
//!
//! * **Bounded concurrency** — probing thousands of endpoints one-at-a-time is
//!   slow, but all-at-once would hammer the network and get you rate-limited.
//!   `buffer_unordered(n)` keeps at most `n` probes in flight at once.
//! * **Per-item errors that don't abort the batch** — one dead endpoint is data,
//!   not a fatal error, so we capture each probe's `Result` instead of `?`-ing.
//! * **Calling a sibling crate** — `scoring::score(&view)` is an ordinary
//!   function call into our pure library; no I/O crosses that boundary.

mod clustering;
mod metadata;
mod netguard;
mod observe;
mod scoring_step;
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

    // 4. Re-cluster across ALL agents (needs the global graph) and persist.
    let clusters = clustering::detect(db).await?;
    tracing::info!("detected {} suspicious clusters", clusters.len());
    db.write_clusters(&clusters).await?;

    // 5. Score every agent from freshly-enriched data and store the results.
    let scored = scoring_step::score_all(db).await?;
    tracing::info!("scored {scored} agents");

    Ok(())
}
