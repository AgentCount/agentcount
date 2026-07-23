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

use std::time::{Duration, Instant};

use crate::metadata::AgentStub;

/// The outcome of a single probe. Richer than a `bool`, and `match`-friendly.
#[derive(Debug, Clone)]
pub enum ProbeOutcome {
    /// Endpoint responded, and the response looked like a valid agent-card.
    Healthy { latency_ms: u64 },
    /// Returned an HTTP error status (4xx/5xx). We keep the `status` for logging
    /// and debugging even though the DB only stores the coarse outcome label.
    HttpError {
        #[allow(dead_code)]
        status: u16,
    },
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
        matches!(self, ProbeOutcome::Healthy { .. })
    }

    /// A short, stable label stored in the `probe_history.outcome` column. Using
    /// a method (rather than scattering string literals) keeps the DB values and
    /// the enum in lockstep.
    pub fn label(&self) -> &'static str {
        match self {
            ProbeOutcome::Healthy { .. } => "healthy",
            ProbeOutcome::HttpError { .. } => "http_error",
            ProbeOutcome::Timeout => "timeout",
            ProbeOutcome::Unreachable => "unreachable",
        }
    }

    /// The measured latency in milliseconds, if the probe succeeded. Stored in
    /// the nullable `latency_ms` column — `None` becomes SQL `NULL`.
    pub fn latency_ms(&self) -> Option<i32> {
        match self {
            ProbeOutcome::Healthy { latency_ms } => Some(*latency_ms as i32),
            _ => None,
        }
    }
}

/// Probe one agent's endpoint once and classify the result.
///
/// Note the return type is `ProbeOutcome`, not `Result<ProbeOutcome>`: a failure
/// to connect isn't an *error* to this function — it's the answer. We convert
/// network errors into `Unreachable`/`Timeout` variants at this boundary so the
/// caller never has to `?` and can just record whatever came back.
pub async fn probe(agent: &AgentStub) -> ProbeOutcome {
    let url = format!("https://{}/.well-known/agent.json", agent.domain);

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        // If we can't even build a client, treat it as unreachable rather than
        // panicking — the daemon must keep running.
        Err(_) => return ProbeOutcome::Unreachable,
    };

    // `Instant::now()` is a monotonic timer (unlike a wall clock), the right tool
    // for measuring an elapsed duration.
    let start = Instant::now();

    // This `match` is the heart of the boundary: every possible network result is
    // funnelled into exactly one tidy domain variant.
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => ProbeOutcome::Healthy {
            latency_ms: start.elapsed().as_millis() as u64,
        },
        Ok(resp) => ProbeOutcome::HttpError {
            status: resp.status().as_u16(),
        },
        Err(e) if e.is_timeout() => ProbeOutcome::Timeout,
        Err(_) => ProbeOutcome::Unreachable,
    }
}
