//! `Prober`: the one shared, guarded HTTP client. This is where the
//! politeness commitments in `METHODOLOGY.md` §6 actually get enforced —
//! everything below exists because roughly 23,744 of our ~60,000 requests
//! land on a server belonging to someone who has never heard of us, and hosts
//! repeat heavily in that population.
//!
//! **Redirects.** Day 1's enricher disabled redirects outright, which made
//! `final_url` meaningless and would fail any agent sitting behind a
//! legitimate 301. Here we follow up to [`MAX_REDIRECTS`] hops ourselves
//! (`reqwest`'s automatic redirect handling is turned off), re-running the
//! netguard's SSRF check on EVERY hop — the initial URL and each `Location`
//! we're about to follow — because following redirects without re-validating
//! each one would reopen exactly the SSRF hole the guard exists to close: a
//! public, allowed host could 302 us to `http://169.254.169.254/`.
//!
//! **Concurrency.** Two independent caps: a per-host `Semaphore` (capped at
//! [`PER_HOST_CAP`]) keyed by hostname behind a `Mutex`, and a global
//! `Semaphore` (default [`DEFAULT_GLOBAL_CONCURRENCY`], overridable via
//! `PROBE_CONCURRENCY`). The per-host cap is the one that matters for
//! politeness — Day 1 saw six consecutive agents on `owockibot.xyz`, and a
//! global cap alone would happily put all 8 of its permits on one small
//! server at once.
//!
//! **Testing against a local mock.** wiremock binds `127.0.0.1`, which the
//! netguard correctly refuses (loopback is never public). So the mechanics
//! tests in this file (200/402/404/redirect/oversized/timeout/per-host-cap)
//! call the crate-internal [`Prober::fetch_http`] with `validate_hops:
//! false`, bypassing ONLY the netguard step — the guard's own behavior is
//! exhaustively tested in `netguard.rs`, unchanged. `Prober::fetch`, the
//! production entry point, always validates.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::netguard;
use crate::resolve::{self, Target};
use crate::robots::{RobotsCache, RobotsDecision};

/// Refuse to keep more than this many response bytes, from any single
/// response (robots.txt included) — plenty for any real agent card, small
/// enough that a hostile endpoint can't OOM the prober. On overflow we keep
/// the first `MAX_BODY_BYTES` and mark the outcome `truncated`, rather than
/// discarding it: a truncated body is OUR limit, not the agent's malformity,
/// so a later rung must report `error`, never judge the JSON invalid.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Per-host concurrency cap. The requirement a global cap does not satisfy.
pub const PER_HOST_CAP: usize = 2;

/// Global concurrency cap, absent `PROBE_CONCURRENCY`.
pub const DEFAULT_GLOBAL_CONCURRENCY: usize = 8;

/// Redirects followed before giving up. Politeness, not paranoia: a chain
/// longer than this is unusual for a legitimate endpoint.
pub const MAX_REDIRECTS: u8 = 3;

/// Our product token — the part of the User-Agent before the version, and
/// what `robots.rs` matches `User-agent:` lines against. Defined once, here,
/// so the header string and the robots.txt match never drift apart.
pub(crate) const PRODUCT_TOKEN: &str = "ledgerscope-probe";
const PRODUCT_VERSION: &str = "0.2";

/// Everything one fetch attempt told us — plain data, no verdict. `crates/checks`
/// is the only place that turns this into pass/fail/skipped/error.
#[derive(Debug, Clone)]
pub struct FetchOutcome {
    /// The scheme bucket this URI fell into: `""` (empty), `"data"`,
    /// `"http"`, `"https"`, `"ipfs"`, or the best-effort label of an
    /// unsupported scheme (e.g. `"ftp"`, `"unknown"`).
    pub scheme: String,
    /// The URL we set out to fetch, when there was one (`None` for
    /// `Empty`/`Unsupported`/`Inline`).
    pub request_url: Option<String>,
    /// The URL that actually answered, after following redirects. Distinct
    /// from `request_url` only when a redirect was followed.
    pub final_url: Option<String>,
    pub http_status: Option<u16>,
    pub content_type: Option<String>,
    /// Response headers, name → joined value. `Value::Null` when no HTTP
    /// response was received.
    pub headers: serde_json::Value,
    /// The bytes we kept — for a `data:` URI, the decoded payload; for an
    /// HTTP fetch, the (possibly `truncated`) response body. `body.as_ref()`
    /// carries what would otherwise need a second `inline_bytes` field: when
    /// `scheme == "data"`, `body.as_ref().map(Vec::len)` IS the inline length.
    pub body: Option<Vec<u8>>,
    /// sha256 of exactly the bytes in `body` (i.e. over the truncated bytes
    /// when `truncated` is set, not some untruncated original we didn't keep).
    pub body_sha256: Option<String>,
    /// Set when `body` was cut off at `MAX_BODY_BYTES`. OUR limit, not the
    /// agent's malformity — see the module doc.
    pub truncated: bool,
    /// Anything that kept us from getting a usable HTTP response: SSRF
    /// rejection, robots.txt disallow (explicit or because robots.txt itself
    /// was unreachable), timeout, connection failure, too many redirects.
    /// Plain text, not a verdict — `crates/checks` decides what each string
    /// means for pass/fail/error.
    pub error: Option<String>,
    /// Wall-clock time for the whole attempt (robots.txt check included),
    /// success or failure. `None` only when no attempt was made at all
    /// (`Empty`/`Unsupported`/`Inline`).
    pub elapsed_ms: Option<u32>,
    /// Which IPFS gateway served this, when one did — so a reader can tell
    /// an agent's failure from our gateway's.
    pub via_gateway: Option<String>,
}

impl FetchOutcome {
    fn base(scheme: impl Into<String>) -> Self {
        FetchOutcome {
            scheme: scheme.into(),
            request_url: None,
            final_url: None,
            http_status: None,
            content_type: None,
            headers: serde_json::Value::Null,
            body: None,
            body_sha256: None,
            truncated: false,
            error: None,
            elapsed_ms: None,
            via_gateway: None,
        }
    }
}

/// A response we've already read fully (up to the body cap), permits and all
/// released. `pub(crate)` so `robots.rs` (which shares `guarded_send`) can
/// read it directly.
pub(crate) struct RawResponse {
    pub(crate) status: u16,
    pub(crate) headers: reqwest::header::HeaderMap,
    pub(crate) body: Vec<u8>,
    pub(crate) truncated: bool,
    pub(crate) location: Option<String>,
}

/// Why a request never produced a `RawResponse`.
pub(crate) enum SendError {
    Timeout,
    Connection(String),
}

/// The one shared HTTP client, plus the state that makes it polite: per-host
/// and global concurrency caps, and a process-lifetime robots.txt cache.
pub struct Prober {
    client: reqwest::Client,
    ipfs_gateway: String,
    total_timeout: Duration,
    global: Arc<Semaphore>,
    per_host: Mutex<HashMap<String, Arc<Semaphore>>>,
    pub(crate) robots: RobotsCache,
}

fn global_concurrency() -> usize {
    std::env::var("PROBE_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_GLOBAL_CONCURRENCY)
}

impl Prober {
    /// `contact_url` is the published contact string from `METHODOLOGY.md`
    /// (e.g. `"https://ledgerscope.io/methodology; contact: probes@ledgerscope.io"`)
    /// — passed in, never hardcoded here, so METHODOLOGY.md stays the single
    /// source and this crate can't drift from what it promises. The full
    /// User-Agent (`ledgerscope-probe/0.2 (+<contact_url>)`) is assembled in
    /// exactly one place: [`Self::build`].
    pub fn new(contact_url: &str, ipfs_gateway: &str) -> anyhow::Result<Self> {
        Self::build(
            contact_url,
            ipfs_gateway,
            Duration::from_secs(5),
            Duration::from_secs(10),
        )
    }

    fn build(
        contact_url: &str,
        ipfs_gateway: &str,
        connect_timeout: Duration,
        total_timeout: Duration,
    ) -> anyhow::Result<Self> {
        let user_agent = format!("{PRODUCT_TOKEN}/{PRODUCT_VERSION} (+{contact_url})");
        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            .connect_timeout(connect_timeout)
            .timeout(total_timeout)
            // We handle redirects ourselves — see the module doc.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            ipfs_gateway: ipfs_gateway.to_string(),
            total_timeout,
            global: Arc::new(Semaphore::new(global_concurrency())),
            per_host: Mutex::new(HashMap::new()),
            robots: RobotsCache::new(),
        })
    }

    /// Test-only constructor: short timeouts so the timeout test doesn't
    /// take ten seconds, routed through the SAME `build` that assembles the
    /// User-Agent — no second place constructs it.
    #[cfg(test)]
    pub(crate) fn new_for_test(connect_timeout: Duration, total_timeout: Duration) -> Self {
        Self::build(
            "https://ledgerscope.io/methodology; contact: probes@ledgerscope.io",
            "https://ipfs.io/ipfs/",
            connect_timeout,
            total_timeout,
        )
        .expect("test client must build")
    }

    fn host_semaphore(&self, host: &str) -> Arc<Semaphore> {
        let mut map = self.per_host.lock().expect("per_host mutex poisoned");
        map.entry(host.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(PER_HOST_CAP)))
            .clone()
    }

    /// Fetch one agent's declared URI. Infallible by design: every failure
    /// mode is a recorded [`FetchOutcome`], never an `Err` that would abort a
    /// 60,000-agent sweep.
    pub async fn fetch(&self, uri: &str) -> FetchOutcome {
        match resolve::resolve(uri, &self.ipfs_gateway) {
            Target::Empty => FetchOutcome::base(""),
            Target::Unsupported { scheme } => FetchOutcome::base(scheme),
            Target::Inline { bytes } => Self::inline_outcome(bytes),
            Target::Http { url, via_gateway } => self.fetch_http(url, via_gateway, true).await,
        }
    }

    fn inline_outcome(bytes: Vec<u8>) -> FetchOutcome {
        let mut out = FetchOutcome::base("data");
        let (kept, truncated) = cap_bytes(bytes);
        out.body_sha256 = Some(sha256_hex(&kept));
        out.truncated = truncated;
        out.body = Some(kept);
        out.elapsed_ms = Some(0);
        out
    }

    /// The guarded HTTP path for an `Http` target: netguard-validate,
    /// robots-gate, fetch, follow redirects up to [`MAX_REDIRECTS`] hops
    /// (re-validating and re-permit-acquiring each one). `validate_hops` is
    /// crate-internal and `false` only in this file's own mechanics tests —
    /// see the module doc for why.
    pub(crate) async fn fetch_http(
        &self,
        initial: url::Url,
        via_gateway: Option<String>,
        validate_hops: bool,
    ) -> FetchOutcome {
        let scheme_label = if via_gateway.is_some() {
            "ipfs".to_string()
        } else {
            initial.scheme().to_string()
        };
        let start = Instant::now();

        let mut out = FetchOutcome::base(scheme_label);
        out.request_url = Some(initial.to_string());
        out.via_gateway = via_gateway;

        let mut current = initial;
        let mut robots_checked = false;

        for hop in 0..=MAX_REDIRECTS {
            if validate_hops && let Err(reason) = self.validate_hop(&current).await {
                out.error = Some(format!("ssrf_blocked: {reason}"));
                out.elapsed_ms = Some(elapsed_ms(start));
                return out;
            }

            if !robots_checked {
                robots_checked = true;
                match self.check_robots(&current, current.path()).await {
                    RobotsDecision::Allowed => {}
                    RobotsDecision::Disallowed => {
                        out.error = Some("robots_disallowed".into());
                        out.elapsed_ms = Some(elapsed_ms(start));
                        return out;
                    }
                    RobotsDecision::Unavailable(reason) => {
                        out.error = Some(format!("robots_unavailable: {reason}"));
                        out.elapsed_ms = Some(elapsed_ms(start));
                        return out;
                    }
                }
            }

            let resp = match self.guarded_send(&current).await {
                Ok(r) => r,
                Err(SendError::Timeout) => {
                    out.error = Some("timeout".into());
                    out.elapsed_ms = Some(elapsed_ms(start));
                    return out;
                }
                Err(SendError::Connection(e)) => {
                    out.error = Some(format!("connection_failed: {e}"));
                    out.elapsed_ms = Some(elapsed_ms(start));
                    return out;
                }
            };

            let is_redirect = (300..400).contains(&resp.status);
            if is_redirect
                && hop < MAX_REDIRECTS
                && let Some(loc) = &resp.location
            {
                match current.join(loc) {
                    Ok(next) => {
                        current = next;
                        continue;
                    }
                    Err(e) => {
                        out.error = Some(format!("bad_redirect_location: {e}"));
                        out.elapsed_ms = Some(elapsed_ms(start));
                        return out;
                    }
                }
            }
            // 3xx with no (or unusable) Location header: nothing to
            // follow, fall through and record it as the terminal response.

            out.final_url = Some(current.to_string());
            out.http_status = Some(resp.status);
            out.content_type = resp
                .headers
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            out.headers = headers_to_json(&resp.headers);
            out.truncated = resp.truncated;
            if is_redirect {
                // Exhausted MAX_REDIRECTS and it's still a redirect — OUR
                // limit, so `error`, not a bare 3xx status left to be
                // misjudged as the agent's answer.
                out.error = Some("too_many_redirects".into());
            } else {
                out.body_sha256 = Some(sha256_hex(&resp.body));
            }
            out.body = Some(resp.body);
            out.elapsed_ms = Some(elapsed_ms(start));
            return out;
        }
        unreachable!("the loop above always returns before exhausting 0..=MAX_REDIRECTS")
    }

    async fn validate_hop(&self, url: &url::Url) -> Result<(), String> {
        match netguard::resolve(url.as_str(), &self.ipfs_gateway).await {
            netguard::Resolution::Fetch(_) => Ok(()),
            netguard::Resolution::Reject(reason) => Err(reason),
            netguard::Resolution::Inline(_) => {
                Err("unexpected inline resolution while validating an http(s) hop".into())
            }
        }
    }

    /// Send one GET, gated by the per-host and global semaphores (held for
    /// the whole exchange, body read included — the connection is still
    /// occupying the host's attention while we stream a capped body), and
    /// bounded by `total_timeout` regardless of how `reqwest` internally
    /// scopes its own timeout.
    pub(crate) async fn guarded_send(&self, url: &url::Url) -> Result<RawResponse, SendError> {
        let host = url.host_str().unwrap_or_default().to_string();
        let _host_permit = self
            .host_semaphore(&host)
            .acquire_owned()
            .await
            .expect("semaphore never closes");
        let _global_permit = self
            .global
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore never closes");

        let attempt = async {
            let resp = self.client.get(url.clone()).send().await.map_err(|e| {
                if e.is_timeout() {
                    SendError::Timeout
                } else {
                    SendError::Connection(e.to_string())
                }
            })?;
            let status = resp.status().as_u16();
            let headers = resp.headers().clone();
            let location = headers
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let (body, truncated) = read_body_capped(resp)
                .await
                .map_err(|e| SendError::Connection(e.to_string()))?;
            Ok(RawResponse {
                status,
                headers,
                body,
                truncated,
                location,
            })
        };

        match tokio::time::timeout(self.total_timeout, attempt).await {
            Ok(result) => result,
            Err(_) => Err(SendError::Timeout),
        }
    }
}

fn elapsed_ms(start: Instant) -> u32 {
    start.elapsed().as_millis().min(u32::MAX as u128) as u32
}

fn cap_bytes(mut bytes: Vec<u8>) -> (Vec<u8>, bool) {
    if bytes.len() > MAX_BODY_BYTES {
        bytes.truncate(MAX_BODY_BYTES);
        (bytes, true)
    } else {
        (bytes, false)
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn headers_to_json(headers: &reqwest::header::HeaderMap) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for name in headers.keys() {
        let values: Vec<&str> = headers
            .get_all(name)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect();
        map.insert(
            name.to_string(),
            serde_json::Value::String(values.join(", ")),
        );
    }
    serde_json::Value::Object(map)
}

/// Read at most `MAX_BODY_BYTES`. On overflow, keep exactly the first
/// `MAX_BODY_BYTES` and report `truncated = true` rather than discarding
/// everything — a truncated body is still evidence, and it must read as OUR
/// limit, not the agent's malformed JSON.
async fn read_body_capped(resp: reqwest::Response) -> Result<(Vec<u8>, bool), reqwest::Error> {
    let mut out = Vec::new();
    let mut truncated = false;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let remaining = MAX_BODY_BYTES - out.len();
        if chunk.len() <= remaining {
            out.extend_from_slice(&chunk);
        } else {
            out.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
    }
    Ok((out, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    fn fast_prober() -> Prober {
        Prober::new_for_test(Duration::from_secs(2), Duration::from_secs(2))
    }

    async fn allow_robots(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(404))
            .mount(server)
            .await;
    }

    fn target_url(server: &MockServer, path: &str) -> url::Url {
        url::Url::parse(&format!("{}{}", server.uri(), path)).unwrap()
    }

    #[tokio::test]
    async fn a_200_with_json_is_recorded_plainly() {
        let server = MockServer::start().await;
        allow_robots(&server).await;
        Mock::given(method("GET"))
            .and(path("/card.json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"name": "agent"})),
            )
            .mount(&server)
            .await;

        let prober = fast_prober();
        let out = prober
            .fetch_http(target_url(&server, "/card.json"), None, false)
            .await;

        assert_eq!(out.http_status, Some(200));
        assert!(out.error.is_none());
        assert!(!out.truncated);
        let body: serde_json::Value = serde_json::from_slice(out.body.as_ref().unwrap()).unwrap();
        assert_eq!(body["name"], "agent");
        assert!(out.body_sha256.is_some());
        assert!(out.elapsed_ms.is_some());
        assert_eq!(out.final_url, out.request_url);
    }

    #[tokio::test]
    async fn a_402_is_recorded_with_no_special_casing() {
        let server = MockServer::start().await;
        allow_robots(&server).await;
        Mock::given(method("GET"))
            .and(path("/card.json"))
            .respond_with(
                ResponseTemplate::new(402).set_body_json(serde_json::json!({"price": "0.01"})),
            )
            .mount(&server)
            .await;

        let prober = fast_prober();
        let out = prober
            .fetch_http(target_url(&server, "/card.json"), None, false)
            .await;

        // Just the status and body, recorded like any other — rung 2 decides
        // what 402 means, not this crate.
        assert_eq!(out.http_status, Some(402));
        assert!(out.error.is_none());
        let body: serde_json::Value = serde_json::from_slice(out.body.as_ref().unwrap()).unwrap();
        assert_eq!(body["price"], "0.01");
    }

    #[tokio::test]
    async fn a_404_is_recorded_plainly() {
        let server = MockServer::start().await;
        allow_robots(&server).await;
        Mock::given(method("GET"))
            .and(path("/card.json"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let prober = fast_prober();
        let out = prober
            .fetch_http(target_url(&server, "/card.json"), None, false)
            .await;

        assert_eq!(out.http_status, Some(404));
        assert!(
            out.error.is_none(),
            "a 404 is the agent's answer, not our error"
        );
    }

    #[tokio::test]
    async fn a_redirect_chain_is_followed_and_final_url_recorded() {
        let server = MockServer::start().await;
        allow_robots(&server).await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/next"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/next"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/final"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/final"))
            .respond_with(ResponseTemplate::new(200).set_body_string("landed"))
            .mount(&server)
            .await;

        let prober = fast_prober();
        let out = prober
            .fetch_http(target_url(&server, "/start"), None, false)
            .await;

        assert_eq!(out.http_status, Some(200));
        assert!(out.error.is_none());
        assert_eq!(out.body.as_deref(), Some(b"landed".as_slice()));
        assert!(out.final_url.as_deref().unwrap().ends_with("/final"));
        assert!(out.request_url.as_deref().unwrap().ends_with("/start"));
        assert_ne!(out.final_url, out.request_url);
    }

    #[tokio::test]
    async fn a_redirect_chain_longer_than_the_cap_is_an_error_not_a_verdict() {
        let server = MockServer::start().await;
        allow_robots(&server).await;
        for (from, to) in [("/a", "/b"), ("/b", "/c"), ("/c", "/d"), ("/d", "/e")] {
            Mock::given(method("GET"))
                .and(path(from))
                .respond_with(ResponseTemplate::new(302).insert_header("Location", to))
                .mount(&server)
                .await;
        }

        let prober = fast_prober();
        let out = prober
            .fetch_http(target_url(&server, "/a"), None, false)
            .await;

        assert_eq!(out.error.as_deref(), Some("too_many_redirects"));
    }

    #[tokio::test]
    async fn an_oversized_body_is_truncated_not_discarded() {
        let server = MockServer::start().await;
        allow_robots(&server).await;
        Mock::given(method("GET"))
            .and(path("/big.json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(vec![b'x'; MAX_BODY_BYTES + 4096]),
            )
            .mount(&server)
            .await;

        let prober = fast_prober();
        let out = prober
            .fetch_http(target_url(&server, "/big.json"), None, false)
            .await;

        assert_eq!(out.http_status, Some(200));
        assert!(
            out.error.is_none(),
            "a truncated body is OUR limit, not the agent's malformity"
        );
        assert!(out.truncated);
        assert_eq!(out.body.as_ref().unwrap().len(), MAX_BODY_BYTES);
    }

    #[tokio::test]
    async fn a_slow_endpoint_times_out() {
        let server = MockServer::start().await;
        allow_robots(&server).await;
        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
            .mount(&server)
            .await;

        // A very short total timeout so this test stays fast.
        let prober = Prober::new_for_test(Duration::from_millis(200), Duration::from_millis(200));
        let out = prober
            .fetch_http(target_url(&server, "/slow"), None, false)
            .await;

        assert_eq!(out.error.as_deref(), Some("timeout"));
        assert!(out.http_status.is_none());
        assert!(out.elapsed_ms.is_some());
    }

    /// The per-host cap must be demonstrated by a test, not merely
    /// configured — a cap you have not watched hold is a cap you are hoping
    /// for. This fires more concurrent fetches at one host than the cap
    /// allows and records the actual high-water mark of requests the mock
    /// server saw in flight at once (any path — robots.txt included).
    struct ConcurrencyTracker {
        current: Arc<AtomicUsize>,
        max_seen: Arc<AtomicUsize>,
    }

    impl Respond for ConcurrencyTracker {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            if req.url.path() == "/robots.txt" {
                return ResponseTemplate::new(404);
            }
            let now = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_seen.fetch_max(now, Ordering::SeqCst);
            // Block this responder thread briefly so overlapping requests
            // are actually overlapping when measured, not serialized by
            // being fast.
            std::thread::sleep(Duration::from_millis(120));
            self.current.fetch_sub(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_string("ok")
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn no_more_than_two_requests_are_ever_concurrently_in_flight_against_one_host() {
        let server = MockServer::start().await;
        let max_seen = Arc::new(AtomicUsize::new(0));
        let tracker = ConcurrencyTracker {
            current: Arc::new(AtomicUsize::new(0)),
            max_seen: max_seen.clone(),
        };
        Mock::given(method("GET"))
            .respond_with(tracker)
            .mount(&server)
            .await;

        let prober = Arc::new(fast_prober());
        let url = target_url(&server, "/card.json");

        let mut handles = Vec::new();
        for _ in 0..6 {
            let prober = prober.clone();
            let url = url.clone();
            handles.push(tokio::spawn(async move {
                prober.fetch_http(url, None, false).await
            }));
        }
        for h in handles {
            let out = h.await.unwrap();
            assert_eq!(out.http_status, Some(200));
        }

        assert!(
            max_seen.load(Ordering::SeqCst) <= PER_HOST_CAP,
            "observed {} requests concurrently in flight against one host; cap is {PER_HOST_CAP}",
            max_seen.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn user_agent_matches_the_published_methodology_string() {
        // METHODOLOGY.md line 301 is the single source for this string;
        // Prober::new's contact_url parameter must reproduce it exactly.
        let contact_url = "https://ledgerscope.io/methodology; contact: probes@ledgerscope.io";
        let ua = format!("{PRODUCT_TOKEN}/{PRODUCT_VERSION} (+{contact_url})");
        assert_eq!(
            ua,
            "ledgerscope-probe/0.2 (+https://ledgerscope.io/methodology; contact: probes@ledgerscope.io)"
        );
    }
}
