//! Probe whether an agent's endpoint is actually alive.
//!
//! Liveness is a cheap reality check that's hard to fake at scale: a bot farm
//! can mint a thousand on-chain identities for pennies, but keeping a thousand
//! real endpoints healthy and responsive is real work. Each probe result is
//! written to the database; the scorer later turns the *history* of probes into
//! the liveness sub-score (success rate over time).
//!
//! Rust concept spotlight: **modelling outcomes with an enum instead of a
//! bool.** "Up or down" loses information — was it a timeout? a 500? a valid
//! response but garbage body? A small enum captures the distinction so we can
//! store and reason about *why* something was unreachable.

use crate::metadata::AgentStub;

/// The outcome of a single probe. Richer than a `bool`, and `match`-friendly.
#[derive(Debug, Clone)]
pub enum ProbeOutcome {
    /// Endpoint responded, and the response looked like a valid agent-card.
    Healthy { latency_ms: u64 },
    /// Responded, but the body wasn't the valid agent-card we expected.
    RespondedButInvalid { status: u16 },
    /// Returned an HTTP error status (4xx/5xx).
    HttpError { status: u16 },
    /// Never responded in time.
    Timeout,
    /// Couldn't even connect (DNS failure, connection refused, TLS error…).
    Unreachable,
}

impl ProbeOutcome {
    /// Did this probe count as a *successful, valid* contact? The scorer's
    /// liveness sub-score is (roughly) the fraction of probes for which this is
    /// `true`. Centralising the definition here keeps "what counts as up"
    /// consistent everywhere.
    pub fn is_success(&self) -> bool {
        // `matches!` is a compact macro for "does this value match this pattern?"
        // — cleaner than a full `match` when you only care about one arm.
        matches!(self, ProbeOutcome::Healthy { .. })
    }
}

/// Probe one agent's endpoint once and classify the result.
///
/// Note the return type is `ProbeOutcome`, not `Result<ProbeOutcome>`: a
/// failure to connect isn't an *error* to this function — it's the answer. We
/// deliberately convert network errors into `Unreachable`/`Timeout` variants so
/// the caller never has to `?` and can just record whatever came back.
pub async fn probe(agent: &AgentStub) -> ProbeOutcome {
    // Sketch:
    //     let url = format!("https://{}/.well-known/agent.json", agent.domain);
    //     let client = reqwest::Client::new();
    //     let start = std::time::Instant::now();  // monotonic timer, fine to use
    //     match client.get(&url)
    //         .timeout(std::time::Duration::from_secs(10))
    //         .send().await
    //     {
    //         Ok(resp) if resp.status().is_success() => {
    //             // Optionally validate the body is a real agent-card here.
    //             ProbeOutcome::Healthy { latency_ms: start.elapsed().as_millis() as u64 }
    //         }
    //         Ok(resp) => ProbeOutcome::HttpError { status: resp.status().as_u16() },
    //         Err(e) if e.is_timeout() => ProbeOutcome::Timeout,
    //         Err(_) => ProbeOutcome::Unreachable,
    //     }
    //
    // See how the `match` turns every possible network result into one of our
    // outcome variants? That's the pattern: convert messy external errors into a
    // tidy domain enum right at the boundary.

    let _ = agent;
    todo!("HTTP-probe the endpoint and classify the result into a ProbeOutcome")
}
