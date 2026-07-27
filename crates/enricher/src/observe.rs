//! One observation per agent: resolve its `agentURI`, obtain the card (over the
//! network or inline), and record BOTH the liveness outcome and the metadata
//! snapshot from a single event.
//!
//! The old design fetched a `https://{domain}/.well-known/agent.json` URL it
//! constructed. Real ERC-8004 agents instead publish an `agentURI` that may be
//! an https URL, an inline `data:` card, an `ipfs://` reference, or garbage.
//! [`netguard::resolve`] turns that into a fetch plan; we act on it here.
//!
//! Two rules worth their comments:
//! * **HTTP 402 is ALIVE.** For an x402-payable endpoint, "Payment Required"
//!   is the most alive possible answer.
//! * **Bodies are capped.** A hostile card can be 10 GB; we read at most
//!   `MAX_BODY_BYTES` and treat overflow as a malformed response.

use std::time::{Duration, Instant};

use futures::StreamExt;
use sha2::{Digest, Sha256};

use crate::metadata::AgentStub;
use crate::netguard::{self, Resolution};

/// Refuse to read response bodies beyond this (1 MiB): plenty for any real
/// agent card, small enough that a hostile endpoint can't OOM the enricher.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Cap the `url` we record in provenance — a `data:` URI can be many KB.
const MAX_STORED_URL: usize = 512;

/// The outcome of one observation. Richer than a bool: WHY it failed is data.
#[derive(Debug, Clone)]
pub enum ProbeOutcome {
    /// Reachable and answering as itself (HTTP 2xx), or an inline card decoded.
    Healthy { latency_ms: u64 },
    /// HTTP 402 — alive AND asking for payment (the x402 signal).
    PaymentRequired { latency_ms: u64 },
    /// Any other HTTP status.
    HttpError { status: u16 },
    Timeout,
    Unreachable,
    /// The URI couldn't be turned into a safe, fetchable target (empty,
    /// malformed, unsupported scheme, or a non-public address). This is an
    /// observation about the AGENT, not a pipeline error.
    Rejected { reason: String },
}

impl ProbeOutcome {
    /// "Alive" = we obtained the card: an HTTP 2xx/402, or an inline card.
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
    /// `None` when we got the card, the specific reason for a rejection, else
    /// the coarse label. Keeping the reason makes "why couldn't we reach this
    /// agent" answerable from history.
    pub fn error_detail(&self) -> Option<String> {
        match self {
            ProbeOutcome::Healthy { .. } | ProbeOutcome::PaymentRequired { .. } => None,
            ProbeOutcome::Rejected { reason } => Some(format!("rejected: {reason}")),
            other => Some(other.label().to_string()),
        }
    }
}

/// Everything one observation told us. `body`/`body_hash` are Some only when we
/// got bytes (any status — a 402 body often carries x402 payment terms).
pub struct Observation {
    pub agent: AgentStub,
    pub url: String,
    pub outcome: ProbeOutcome,
    pub body: Option<serde_json::Value>,
    pub body_hash: Option<String>,
}

/// The ONE shared HTTP client. reqwest's Client is an Arc'd connection pool —
/// building it per request churns TLS sessions for nothing. Redirects are
/// disabled: combined with the netguard, a public host can't bounce us to a
/// private one.
pub fn build_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("ledgerscope-observer/0.1 (+https://ledgerscope.example/methodology)")
        .build()?)
}

/// Observe one agent: resolve its URI, obtain the card, classify. Infallible by
/// design — every failure mode is a recorded outcome, not an `Err`.
pub async fn observe(
    client: &reqwest::Client,
    ipfs_gateway: &str,
    agent: &AgentStub,
) -> Observation {
    let stored_url: String = agent.agent_uri.chars().take(MAX_STORED_URL).collect();

    match netguard::resolve(&agent.agent_uri, ipfs_gateway).await {
        Resolution::Reject(reason) => Observation {
            agent: agent.clone(),
            url: stored_url,
            outcome: ProbeOutcome::Rejected { reason },
            body: None,
            body_hash: None,
        },
        // An inline `data:` card: we already have the bytes, no network. It
        // "resolved", so it counts as alive with zero latency.
        Resolution::Inline(bytes) => {
            let (body, body_hash) = digest_body(Some(bytes));
            Observation {
                agent: agent.clone(),
                url: stored_url,
                outcome: ProbeOutcome::Healthy { latency_ms: 0 },
                body,
                body_hash,
            }
        }
        Resolution::Fetch(url) => fetch(client, agent, stored_url, url).await,
    }
}

/// Fetch a network card and classify the result.
async fn fetch(
    client: &reqwest::Client,
    agent: &AgentStub,
    stored_url: String,
    url: url::Url,
) -> Observation {
    let start = Instant::now();
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) if e.is_timeout() => {
            return Observation { agent: agent.clone(), url: stored_url, outcome: ProbeOutcome::Timeout, body: None, body_hash: None };
        }
        Err(_) => {
            return Observation { agent: agent.clone(), url: stored_url, outcome: ProbeOutcome::Unreachable, body: None, body_hash: None };
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
    let (body, body_hash) = digest_body(body_bytes);
    Observation { agent: agent.clone(), url: stored_url, outcome, body, body_hash }
}

/// Hash raw body bytes and parse them as JSON if possible. Non-JSON bodies are
/// still an observation (hash recorded); we just can't archive them as JSONB.
fn digest_body(bytes: Option<Vec<u8>>) -> (Option<serde_json::Value>, Option<String>) {
    match bytes {
        Some(b) => {
            let hash = format!("{:x}", Sha256::digest(&b));
            (serde_json::from_slice::<serde_json::Value>(&b).ok(), Some(hash))
        }
        None => (None, None),
    }
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

    fn stub(agent_uri: &str) -> AgentStub {
        AgentStub { chain: "base".into(), agent_id: 1, agent_uri: agent_uri.into() }
    }

    /// wiremock binds 127.0.0.1 — which the netguard rightly refuses. Tests
    /// exercise the fetch mechanics directly against the mock URL instead.
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
        assert_eq!(body.unwrap()["price"], "0.01");
    }

    #[tokio::test]
    async fn oversized_bodies_are_refused() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/.well-known/agent.json"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; MAX_BODY_BYTES + 1]))
            .mount(&server).await;

        let (outcome, body) = fetch_outcome(&server).await;
        assert!(outcome.is_alive());
        assert!(body.is_none(), "oversized body must not be stored");
    }

    #[tokio::test]
    async fn inline_data_uri_resolves_without_network() {
        let client = build_client().unwrap();
        // base64 of {"name":"inline"}
        let obs = observe(
            &client,
            "https://ipfs.io/ipfs/",
            &stub("data:application/json;base64,eyJuYW1lIjoiaW5saW5lIn0="),
        )
        .await;
        assert!(obs.outcome.is_alive());
        assert_eq!(obs.body.unwrap()["name"], "inline");
    }

    #[tokio::test]
    async fn malformed_and_private_uris_are_rejected_as_data() {
        let client = build_client().unwrap();
        for uri in ["undefined/agents/1/agent-card/v1", "", "http://127.0.0.1/card.json"] {
            let obs = observe(&client, "https://ipfs.io/ipfs/", &stub(uri)).await;
            assert_eq!(obs.outcome.label(), "rejected", "{uri:?}");
            assert!(!obs.outcome.is_alive());
        }
    }
}
