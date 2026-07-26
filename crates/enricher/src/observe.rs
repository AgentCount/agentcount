//! One observation per agent: a single HTTPS fetch that yields BOTH the
//! liveness outcome and the metadata snapshot.
//!
//! The old design fetched the same URL twice (once to parse the card, once to
//! "probe"). Under the facts model they are one event: "at time T the endpoint
//! answered like this and served this content". One request, one appended row
//! of history, no duplicate load on the agent.
//!
//! Two probe rules worth their comments:
//! * **HTTP 402 is ALIVE.** For an x402-payable endpoint, "Payment Required"
//!   is the most alive possible answer. A naive health check would report our
//!   most interesting agents as down.
//! * **Bodies are capped.** A hostile agent card can be 10 GB; we read at most
//!   `MAX_BODY_BYTES` and treat overflow as a malformed response.

use std::time::{Duration, Instant};

use futures::StreamExt;
use sha2::{Digest, Sha256};

use crate::metadata::AgentStub;
use crate::netguard;

/// Refuse to read response bodies beyond this (1 MiB): plenty for any real
/// agent card, small enough that a hostile endpoint can't OOM the enricher.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// The outcome of one observation. Richer than a bool: WHY it failed is data.
#[derive(Debug, Clone)]
pub enum ProbeOutcome {
    /// 2xx with a response we could read.
    Healthy { latency_ms: u64 },
    /// HTTP 402 — alive AND asking for payment (the x402 signal).
    PaymentRequired { latency_ms: u64 },
    /// Any other HTTP status.
    HttpError { status: u16 },
    Timeout,
    Unreachable,
    /// The netguard refused the target (bad domain, private address). This is
    /// an observation about the AGENT (it registered an unprobeable domain),
    /// not an error in our pipeline.
    Rejected { reason: String },
}

impl ProbeOutcome {
    /// "Alive" = reachable and answering as itself: 2xx or 402.
    pub fn is_alive(&self) -> bool {
        matches!(self, ProbeOutcome::Healthy { .. } | ProbeOutcome::PaymentRequired { .. })
    }

    /// Stable label stored in probe_history.outcome.
    pub fn label(&self) -> &'static str {
        match self {
            ProbeOutcome::Healthy { .. } => "healthy",
            ProbeOutcome::PaymentRequired { .. } => "payment_required",
            ProbeOutcome::HttpError { .. } => "http_error",
            ProbeOutcome::Timeout => "timeout",
            ProbeOutcome::Unreachable => "unreachable",
            ProbeOutcome::Rejected { .. } => "rejected",
        }
    }

    pub fn latency_ms(&self) -> Option<i32> {
        match self {
            ProbeOutcome::Healthy { latency_ms } | ProbeOutcome::PaymentRequired { latency_ms } => {
                Some(*latency_ms as i32)
            }
            _ => None,
        }
    }

    pub fn http_status(&self) -> Option<i32> {
        match self {
            ProbeOutcome::Healthy { .. } => Some(200),
            ProbeOutcome::PaymentRequired { .. } => Some(402),
            ProbeOutcome::HttpError { status } => Some(*status as i32),
            _ => None,
        }
    }

    /// What to store in `metadata_snapshots.error` for a non-alive outcome:
    /// `None` when the endpoint answered (alive), the specific reason for a
    /// guard rejection, else the coarse label. Keeping the rejection reason
    /// makes "why couldn't we reach this agent" answerable from history.
    pub fn error_detail(&self) -> Option<String> {
        match self {
            ProbeOutcome::Healthy { .. } | ProbeOutcome::PaymentRequired { .. } => None,
            ProbeOutcome::Rejected { reason } => Some(format!("rejected: {reason}")),
            other => Some(other.label().to_string()),
        }
    }
}

/// Everything one fetch told us. `body`/`body_hash` are Some only when we got
/// bytes back (any status — a 402's body often carries x402 payment terms,
/// which is exactly the kind of thing we archive).
pub struct Observation {
    pub agent: AgentStub,
    pub url: String,
    pub outcome: ProbeOutcome,
    pub body: Option<serde_json::Value>,
    pub body_hash: Option<String>,
}

/// The ONE shared HTTP client. reqwest's Client is an Arc'd connection pool —
/// building it per request (the old pattern) churns TLS sessions for nothing.
/// Redirects are disabled: combined with the netguard, a public host can't
/// bounce us to a private one.
pub fn build_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("ledgerscope-observer/0.1 (+https://ledgerscope.example/methodology)")
        .build()?)
}

/// Observe one agent: guard, fetch once, classify, capture body.
/// Infallible by design — every failure mode is a recorded outcome.
pub async fn observe(client: &reqwest::Client, agent: &AgentStub) -> Observation {
    let url = match netguard::check_target(&agent.domain).await {
        Ok(u) => u,
        Err(reason) => {
            return Observation {
                agent: agent.clone(),
                url: format!("https://{}/.well-known/agent.json", agent.domain),
                outcome: ProbeOutcome::Rejected { reason },
                body: None,
                body_hash: None,
            };
        }
    };
    let url_str = url.to_string();
    let start = Instant::now();

    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) if e.is_timeout() => {
            return Observation { agent: agent.clone(), url: url_str, outcome: ProbeOutcome::Timeout, body: None, body_hash: None };
        }
        Err(_) => {
            return Observation { agent: agent.clone(), url: url_str, outcome: ProbeOutcome::Unreachable, body: None, body_hash: None };
        }
    };

    let status = resp.status().as_u16();
    let body_bytes = read_body_limited(resp).await;
    let latency_ms = start.elapsed().as_millis() as u64;

    let outcome = match status {
        200..=299 => ProbeOutcome::Healthy { latency_ms },
        402 => ProbeOutcome::PaymentRequired { latency_ms },
        s => ProbeOutcome::HttpError { status: s },
    };

    let (body, body_hash) = match body_bytes {
        Some(bytes) => {
            let hash = format!("{:x}", Sha256::digest(&bytes));
            // Non-JSON bodies are still an observation (hash recorded); we
            // just can't archive them as JSONB.
            (serde_json::from_slice::<serde_json::Value>(&bytes).ok(), Some(hash))
        }
        None => (None, None),
    };

    Observation { agent: agent.clone(), url: url_str, outcome, body, body_hash }
}

/// Read at most MAX_BODY_BYTES; None if the stream errors or overflows.
async fn read_body_limited(resp: reqwest::Response) -> Option<Vec<u8>> {
    if resp.content_length().is_some_and(|l| l as usize > MAX_BODY_BYTES) {
        return None; // declared oversized — don't even start
    }
    let mut out: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        if out.len() + chunk.len() > MAX_BODY_BYTES {
            return None; // lied about (or omitted) Content-Length
        }
        out.extend_from_slice(&chunk);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn stub(domain: &str) -> AgentStub {
        AgentStub { chain: "base".into(), agent_id: 1, domain: domain.into() }
    }

    /// wiremock binds 127.0.0.1 — which the netguard rightly refuses. Tests
    /// exercise the fetch path directly against the mock URL instead. This
    /// helper mirrors observe()'s post-guard logic via a plain GET.
    async fn fetch_outcome(server: &MockServer) -> (ProbeOutcome, Option<serde_json::Value>) {
        let client = build_client().unwrap();
        let start = Instant::now();
        let resp = client
            .get(format!("{}/.well-known/agent.json", server.uri()))
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let body = read_body_limited(resp).await;
        let latency_ms = start.elapsed().as_millis() as u64;
        let outcome = match status {
            200..=299 => ProbeOutcome::Healthy { latency_ms },
            402 => ProbeOutcome::PaymentRequired { latency_ms },
            s => ProbeOutcome::HttpError { status: s },
        };
        (outcome, body.and_then(|b| serde_json::from_slice(&b).ok()))
    }

    #[tokio::test]
    async fn a_402_is_alive_and_payable_not_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/.well-known/agent.json"))
            .respond_with(ResponseTemplate::new(402).set_body_json(serde_json::json!({"price": "0.01"})))
            .mount(&server).await;

        let (outcome, body) = fetch_outcome(&server).await;
        assert!(outcome.is_alive(), "402 must count as alive");
        assert_eq!(outcome.label(), "payment_required");
        // The 402 body (x402 payment terms) is captured, not discarded.
        assert_eq!(body.unwrap()["price"], "0.01");
    }

    #[tokio::test]
    async fn oversized_bodies_are_refused() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/.well-known/agent.json"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; MAX_BODY_BYTES + 1]))
            .mount(&server).await;

        let (outcome, body) = fetch_outcome(&server).await;
        assert!(outcome.is_alive()); // it answered — that's still liveness
        assert!(body.is_none(), "oversized body must not be stored");
    }

    #[tokio::test]
    async fn private_domains_are_rejected_as_data() {
        let client = build_client().unwrap();
        let obs = observe(&client, &stub("127.0.0.1")).await;
        assert_eq!(obs.outcome.label(), "rejected");
        assert!(!obs.outcome.is_alive());
    }
}
