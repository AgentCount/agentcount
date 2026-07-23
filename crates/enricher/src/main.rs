//! # enricher — add off-chain reality to on-chain agents
//!
//! The indexer tells us an agent *exists*. The enricher tells us whether it's
//! *real*: does its endpoint respond, does it serve a valid agent-card, and —
//! most importantly — does it look like one honest actor or one node in a farm
//! of coordinated sock-puppets?
//!
//! It reads agents from Postgres, does three kinds of work, and writes the
//! results back for the scorer to consume:
//!   * [`metadata`]   — fetch and parse each agent's agent-card JSON.
//!   * [`liveness`]   — probe the endpoint repeatedly and record uptime.
//!   * [`clustering`] — the Sybil detector: group suspiciously-coordinated agents.
//!
//! ## Rust concepts this crate is here to teach
//!
//! * **Bounded concurrency** — probing thousands of endpoints one-at-a-time is
//!   slow, but all-at-once would hammer the network and get you rate-limited.
//!   The idiom is to run many futures with a *cap* (see the note in `main`).
//! * **`Result` you're allowed to ignore per-item** — when enriching a batch,
//!   one dead endpoint shouldn't abort the whole run. You'll collect per-agent
//!   `Result`s and log failures rather than `?`-ing out.

mod clustering;
mod liveness;
mod metadata;
mod store;

/// Configuration read from the environment at startup.
struct Config {
    /// Postgres connection string.
    database_url: String,
    /// How many endpoints to probe concurrently. A knob you'll tune.
    probe_concurrency: usize,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        //     Ok(Self {
        //         database_url: std::env::var("DATABASE_URL")
        //             .context("DATABASE_URL must be set")?,
        //         probe_concurrency: std::env::var("PROBE_CONCURRENCY")
        //             .ok()
        //             .and_then(|s| s.parse().ok())
        //             .unwrap_or(32), // sensible default
        //     })
        todo!("read DATABASE_URL and an optional PROBE_CONCURRENCY (default 32)")
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    //     tracing_subscriber::fmt().with_env_filter(
    //         tracing_subscriber::EnvFilter::from_default_env()).init();
    //     let config = Config::from_env()?;
    //     let db = store::connect(&config.database_url).await?;

    // The enricher runs as periodic passes rather than a tight tail-follow loop
    // like the indexer. One pass:
    //
    //   1. Load the agents that need (re-)enriching.
    //         let agents = db.load_agents_to_enrich().await?;
    //
    //   2. Probe + fetch metadata for each, with BOUNDED concurrency. The usual
    //      tool is a stream with `.buffer_unordered(n)`, which keeps at most `n`
    //      probes in flight at once — fast, but polite:
    //
    //         use futures::stream::{self, StreamExt};
    //         let results: Vec<_> = stream::iter(agents)
    //             .map(|agent| async move {
    //                 let card  = metadata::fetch_agent_card(&agent).await;
    //                 let alive = liveness::probe(&agent).await;
    //                 (agent, card, alive)
    //             })
    //             .buffer_unordered(config.probe_concurrency)
    //             .collect()
    //             .await;
    //
    //      (That pulls in the `futures` crate — add it when you get here. Note
    //      `async move`: the closure takes ownership of `agent` so the future can
    //      outlive the loop iteration that created it.)
    //
    //   3. Persist per-agent enrichment. A failed probe is data ("it's down"),
    //      not a reason to abort the batch.
    //         db.write_enrichment(&results).await?;
    //
    //   4. Re-run clustering across ALL agents (it needs the global graph) and
    //      persist the clusters + per-agent suspicion signal.
    //         let clusters = clustering::detect(&db).await?;
    //         db.write_clusters(&clusters).await?;

    todo!("run one enrichment pass: load → probe+fetch (bounded) → persist → cluster")
}
