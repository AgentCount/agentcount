//! Fetch and parse an agent's "agent-card".
//!
//! Under ERC-8004 an agent registers a *domain*, and that domain is expected to
//! host a machine-readable description of the agent (an "agent-card", typically
//! JSON at a well-known path). Fetching it tells us what the agent claims to be
//! and gives the liveness check something concrete to validate.
//!
//! Rust concept spotlight: **`serde` deserialization into a typed struct.** We
//! describe the shape we expect with a struct and `#[derive(Deserialize)]`, then
//! reqwest hands the JSON straight into it. Fields we don't care about are
//! ignored; missing optional fields become `None`. If the JSON's types don't
//! match our struct, we get a clean error instead of a landmine.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The lightweight view of an agent that enrichment works from — just enough to
/// know where to fetch its card. Built from a database row in `store.rs`.
///
/// It derives `sqlx::FromRow` so a `SELECT chain, agent_id, domain ...` maps
/// straight into it, and `Clone` so the concurrent probe stage can move a copy
/// into each async job.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentStub {
    pub chain: String,
    /// Postgres `BIGINT` is a signed 64-bit int, so it maps to `i64` (not `u64`).
    /// We convert to `u64` only at the edge where we hand data to `scoring`.
    pub agent_id: i64,
    /// The domain the agent registered on-chain, e.g. "acme-agent.example".
    pub domain: String,
}

/// The subset of an agent-card we care about. `#[derive(Serialize)]` too, so we
/// can store the parsed card back into Postgres as JSONB. serde ignores any JSON
/// keys not mentioned here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    /// Human-readable name the agent advertises.
    pub name: Option<String>,
    /// A description / capabilities blurb.
    pub description: Option<String>,
    /// The endpoint the agent actually serves requests from.
    pub endpoint: Option<String>,
    /// Anything else in the document, kept verbatim so we don't lose data we
    /// haven't modelled yet. `#[serde(flatten)]` folds all un-named keys in here.
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

/// The conventional well-known path for an agent-card. Adjust to match the
/// ERC-8004 spec's actual path if it differs.
fn card_url(domain: &str) -> String {
    format!("https://{domain}/.well-known/agent.json")
}

/// Fetch and parse the agent-card for one agent.
///
/// Returns `Result<AgentCard>`; the caller (in `main.rs`) keeps the whole
/// `Result` rather than `?`-ing it, so a failed fetch becomes "no valid card /
/// endpoint down" data instead of aborting the batch.
pub async fn fetch_agent_card(agent: &AgentStub) -> Result<AgentCard> {
    let url = card_url(&agent.domain);

    // A short, hard timeout is essential: a dead agent must fail fast, not stall
    // the whole batch. Building one client per call is fine here; for high volume
    // you'd build it once and share it.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("building HTTP client")?;

    let card = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?
        .error_for_status() // turn 4xx/5xx into an Err
        .with_context(|| format!("bad status from {url}"))?
        .json::<AgentCard>() // read body AND deserialize in one step
        .await
        .with_context(|| format!("parsing agent-card from {url}"))?;

    Ok(card)
}
