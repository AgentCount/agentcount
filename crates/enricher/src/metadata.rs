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

use anyhow::Result;
use serde::Deserialize;

/// The subset of an agent-card we care about. Add fields as your needs grow;
/// serde ignores any JSON keys not mentioned here.
///
/// `Option<T>` fields are ones that might be absent — serde fills them with
/// `None` rather than failing, which is exactly what you want for optional
/// metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentCard {
    /// Human-readable name the agent advertises.
    pub name: Option<String>,
    /// A description / capabilities blurb.
    pub description: Option<String>,
    /// The endpoint the agent actually serves requests from. This is what
    /// `liveness::probe` will hit.
    pub endpoint: Option<String>,
    /// Anything else in the document, kept verbatim so we don't lose data we
    /// haven't modelled yet. `serde_json::Value` is a "parsed but untyped" JSON
    /// blob — the escape hatch for schema you don't fully know.
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

/// The `AgentStub` is the little bit of on-chain info we already have about an
/// agent (from the indexer) — enough to know where to fetch its card from.
/// Defined here as a placeholder; in the real code it'll come from `store.rs`.
pub struct AgentStub {
    pub agent_id: u64,
    /// The domain the agent registered on-chain, e.g. "acme-agent.example".
    pub domain: String,
}

/// Fetch and parse the agent-card for one agent.
///
/// Returns `Result<AgentCard>`; the caller decides whether a failure means
/// "endpoint down" (data!) or something to retry. We do NOT `?`-abort the whole
/// enrichment pass on one bad fetch.
pub async fn fetch_agent_card(agent: &AgentStub) -> Result<AgentCard> {
    // Convention for where the card lives — adjust to the ERC-8004 spec's actual
    // well-known path:
    //     let url = format!("https://{}/.well-known/agent.json", agent.domain);
    //
    // reqwest + serde in three lines. `.json::<AgentCard>()` both reads the body
    // and deserializes it into our struct in one step:
    //     let client = reqwest::Client::new();
    //     let card = client.get(&url)
    //         .timeout(std::time::Duration::from_secs(10)) // never hang forever
    //         .send().await?
    //         .error_for_status()?      // turn 404/500 into an Err
    //         .json::<AgentCard>().await?;
    //     Ok(card)
    //
    // Give the client a short timeout: a dead agent must fail fast, not stall the
    // whole batch.

    let _ = agent;
    todo!("GET https://{{domain}}/.well-known/agent.json and deserialize into AgentCard")
}
