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
//! **Rate, not just concurrency (2026-08-06).** A concurrency cap bounds how
//! many of our requests a host handles at once; it does not bound how many it
//! handles per second. Two permits against a host answering in 20ms is 100
//! requests a second, indefinitely, and that is how the 2026-08 census
//! collected 19,658 HTTP 429s from `metadata.evoevo.ai` alone — then published
//! them as agents that had stopped resolving. So each host also has a
//! **cadence**: at most one request STARTS per [`DEFAULT_HOST_DELAY`]
//! (overridable via `PROBE_HOST_DELAY_MS`), enforced by a per-host next-start
//! instant that every task claims a slot in before it dials. Four hosts carry
//! 59.2% of every declared endpoint in the census, so this is the limit that
//! decides what those four experience.
//!
//! **`Retry-After` is honoured** on the two statuses it is defined for (429 and
//! 503). The value moves that host's next-start instant forward, so the rest of
//! the sweep backs off the host that asked — it does not retry this request
//! (the sweep is one fetch per agent per run, and a retry would double the
//! traffic of the exact host that just asked us to stop). A backoff longer than
//! [`MAX_HONOURED_RETRY_AFTER`] is clamped to it, because a host asking for an
//! hour would otherwise stall the run's global concurrency behind one lane; the
//! value the origin asked for is still recorded verbatim on the outcome.
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

use serde::Serialize;

use crate::netguard;
use crate::resolve::{self, DataUriDecode, Target};
use crate::robots::{RobotsCache, RobotsDecision};

/// Refuse to keep more than this many response bytes, from any single
/// response (robots.txt included) — plenty for any real agent card, small
/// enough that a hostile endpoint can't OOM the prober. On overflow we keep
/// the first `MAX_BODY_BYTES` and mark the outcome `truncated`, rather than
/// discarding it: a truncated body is OUR limit, not the agent's malformity,
/// so a later rung must report `error`, never judge the JSON invalid.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Per-host concurrency cap: how many of our requests one host may be
/// answering at the same moment. The requirement a global cap does not satisfy.
///
/// Note what this does NOT bound, and what [`DEFAULT_HOST_DELAY`] exists for: a
/// host that answers quickly can be asked again immediately, so two permits are
/// two requests at once but an unbounded number per second.
pub const PER_HOST_CAP: usize = 2;

/// Minimum interval between the START of two requests to the same host, absent
/// `PROBE_HOST_DELAY_MS`. With [`PER_HOST_CAP`] above it, one host sees at most
/// four requests a second from us and at most two in flight.
///
/// 250ms is the number that makes the census's own arithmetic tolerable at both
/// ends. The largest host in the four-chain census is declared by 47,863 agents
/// on BSC; at this cadence that host's share of a sweep takes ~3.3 hours, well
/// inside the 11 hours that sweep already ran, while the 3,391 hosts under the
/// rung-6 sampling budget (which are most of them, and small) see a handful of
/// requests spaced further apart than any human browsing them.
pub const DEFAULT_HOST_DELAY: Duration = Duration::from_millis(250);

/// The longest `Retry-After` we will actually wait out on a host lane.
///
/// Honouring an arbitrary value is not an option: a host asking for an hour
/// would hold one lane — and, through the global semaphore, a share of the
/// whole sweep — for that hour. Clamping is stated rather than silent: the
/// value the origin asked for is recorded on the outcome regardless, so a
/// reader can see we were asked for more than we gave.
pub const MAX_HONOURED_RETRY_AFTER: Duration = Duration::from_secs(120);

/// Global concurrency cap, absent `PROBE_CONCURRENCY`.
pub const DEFAULT_GLOBAL_CONCURRENCY: usize = 8;

/// Redirects followed before giving up. Politeness, not paranoia: a chain
/// longer than this is unusual for a legitimate endpoint.
pub const MAX_REDIRECTS: u8 = 3;

/// Our product token — the part of the User-Agent before the version, and
/// what `robots.rs` matches `User-agent:` lines against. Defined once, here,
/// so the header string and the robots.txt match never drift apart.
pub(crate) const PRODUCT_TOKEN: &str = "agentcount-probe";
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
    /// The backoff the origin asked for, in seconds, when a 429 or a 503
    /// carried a parseable `Retry-After`. Already applied to that host's lane
    /// for the rest of the run — this is the record of what was asked, which is
    /// not the same as what was waited (see [`MAX_HONOURED_RETRY_AFTER`]).
    /// `None` means no header; `Some(0)` means the origin named a moment that
    /// had already passed.
    pub retry_after_secs: Option<u32>,
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
    /// an agent's failure from every gateway's own. Only the WINNING
    /// gateway; see `gateway_attempts` below for the whole chain.
    pub via_gateway: Option<String>,
    /// For a `data:` (or scheme-less raw-JSON) target only: which of P0
    /// FIX 7's five fallback paths produced `body`, and the compression
    /// algorithm when one was involved. `None` for every other scheme.
    pub inline_decode: Option<DataUriDecode>,
    /// P0 FIX 8: every IPFS gateway tried, in order, with each one's own
    /// outcome — set only when `scheme == "ipfs"`. Empty for every other
    /// scheme, and empty for an `ipfs://` URI too malformed to extract a
    /// CID from (that never reaches the gateway loop at all). Lets a reader
    /// see the whole chain, not just whichever gateway (if any) won.
    pub gateway_attempts: Vec<GatewayAttempt>,
}

/// One attempt against one IPFS gateway, as part of the P0 FIX 8
/// fallback chain.
#[derive(Debug, Clone, Serialize)]
pub struct GatewayAttempt {
    pub gateway: String,
    pub http_status: Option<u16>,
    /// Plain text, same non-verdict convention as `FetchOutcome.error`.
    pub error: Option<String>,
}

impl FetchOutcome {
    fn base(scheme: impl Into<String>) -> Self {
        FetchOutcome {
            scheme: scheme.into(),
            request_url: None,
            final_url: None,
            http_status: None,
            retry_after_secs: None,
            content_type: None,
            headers: serde_json::Value::Null,
            body: None,
            body_sha256: None,
            truncated: false,
            error: None,
            elapsed_ms: None,
            via_gateway: None,
            inline_decode: None,
            gateway_attempts: Vec::new(),
        }
    }

    /// Reduce this outcome's raw [`scheme`](FetchOutcome::scheme) label to the
    /// six buckets `checks::ResolvableInput` and the `http_archive.scheme`
    /// column agree on: `"empty"`, `"unsupported"`, `"data"`, `"http"`,
    /// `"https"`, `"ipfs"`.
    ///
    /// `scheme` alone is ambiguous for `data:` and `ipfs://`: a MALFORMED one
    /// carries the SAME label as a genuine one (see
    /// [`crate::Target::Unsupported`] — this crate only knows which scheme it
    /// tried to parse, not whether parsing succeeded), so `scheme == "data"`
    /// alone cannot tell a decoded inline document from a `data:` URI with no
    /// comma separator. `request_url` disambiguates: it is set if, and only
    /// if, an actual HTTP(s) request was attempted — [`Prober::fetch_http`]
    /// sets it as its very first action, before the netguard, the robots
    /// check, or the request itself can fail — so a malformed `ipfs://` (which
    /// never reaches `fetch_http`) is caught here rather than misread as a
    /// passing rung 2. A malformed `data:` URI is caught the same way via
    /// `body`: only a successfully decoded inline payload ever has one.
    ///
    /// **P0 FIX 7:** a `data:` URI declaring an unsupported `enc=` compression
    /// algorithm also carries no `body` (there is nothing decoded to hand
    /// forward) but DOES carry `.error` — that must still land in the `"data"`
    /// bucket, not `"unsupported"`, so rung 2 can tell OUR limitation apart
    /// from a malformed document (see `checks::resolvable`'s `"data"` arm).
    ///
    /// **Lives here rather than in a caller** because there is now more than
    /// one caller: `crates/sweeper` (a census run) and `crates/api`'s
    /// on-demand spot check. Two copies of this reduction would let a spot
    /// check and a census row disagree about which bucket an agent's URI fell
    /// into, and therefore about what rung 2 means for it — the one thing
    /// neither is allowed to do.
    pub fn scheme_bucket(&self) -> String {
        if self.scheme.is_empty() {
            "empty".to_string()
        } else if self.request_url.is_some() {
            // A real HTTP(s) request was attempted (http, https, or ipfs via
            // one of the gateways) — keep whichever of those labels the
            // resolver already assigned.
            self.scheme.clone()
        } else if self.scheme == "data" && (self.body.is_some() || self.error.is_some()) {
            self.scheme.clone()
        } else {
            "unsupported".to_string()
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
    /// The `Retry-After` this response asked for, in seconds, when it was a
    /// 429 or a 503 that carried a parseable one. Already applied to the host's
    /// lane by the time this is returned — this copy is for the record.
    pub(crate) retry_after_secs: Option<u32>,
}

/// One host's share of our politeness: how many of our requests it may be
/// answering at once, and when the next one may start.
///
/// The two are separate limits on purpose (see the module doc): the semaphore
/// bounds simultaneity, the instant bounds rate, and a host that answers in
/// 20ms is only bounded by the second.
struct HostLane {
    permits: Semaphore,
    /// Earliest instant at which a request to this host may be sent. Every
    /// task claims its slot by reading this, taking `max(now, next_start)` for
    /// itself and writing `+ delay` back — so the claims queue up without any
    /// task having to hold a lock across a sleep.
    ///
    /// A `std::sync::Mutex` and not tokio's: it is never held across an await,
    /// and the whole critical section is three instant comparisons.
    next_start: Mutex<Instant>,
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
    /// The gateways an `ipfs://` URI is tried against, in order (P0 FIX 8).
    /// Exactly one gateway reproduces the pre-FIX-8 behaviour; production
    /// use is the three-gateway fallback chain — see `fetch_ipfs_chain`.
    ipfs_gateways: Vec<String>,
    total_timeout: Duration,
    host_delay: Duration,
    global: Arc<Semaphore>,
    per_host: Mutex<HashMap<String, Arc<HostLane>>>,
    pub(crate) robots: RobotsCache,
}

fn global_concurrency() -> usize {
    std::env::var("PROBE_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_GLOBAL_CONCURRENCY)
}

/// The per-host cadence, absent `PROBE_HOST_DELAY_MS`.
///
/// `0` is accepted and means "no cadence limit" — the pre-2026-08-06
/// behaviour. It exists for the tests that measure the delay's own effect,
/// which cannot afford to wait it out, and is a deliberate foot-gun for
/// anything else: the census never sets it.
fn host_delay() -> Duration {
    match std::env::var("PROBE_HOST_DELAY_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        Some(ms) => Duration::from_millis(ms),
        None => DEFAULT_HOST_DELAY,
    }
}

impl Prober {
    /// `contact_url` is the published contact string from `METHODOLOGY.md`
    /// (e.g. `"https://agentcount.ai/methodology; contact: probes@agentcount.ai"`)
    /// — passed in, never hardcoded here, so METHODOLOGY.md stays the single
    /// source and this crate can't drift from what it promises. The full
    /// User-Agent (`agentcount-probe/0.2 (+<contact_url>)`) is assembled in
    /// exactly one place: [`Self::build`]. `ipfs_gateways` is tried in
    /// order for every `ipfs://` URI (P0 FIX 8) — must be non-empty.
    pub fn new(contact_url: &str, ipfs_gateways: &[String]) -> anyhow::Result<Self> {
        anyhow::ensure!(!ipfs_gateways.is_empty(), "ipfs_gateways must not be empty");
        Self::build(
            contact_url,
            ipfs_gateways,
            Duration::from_secs(5),
            Duration::from_secs(10),
            host_delay(),
        )
    }

    fn build(
        contact_url: &str,
        ipfs_gateways: &[String],
        connect_timeout: Duration,
        total_timeout: Duration,
        host_delay: Duration,
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
            ipfs_gateways: ipfs_gateways.to_vec(),
            total_timeout,
            host_delay,
            global: Arc::new(Semaphore::new(global_concurrency())),
            per_host: Mutex::new(HashMap::new()),
            robots: RobotsCache::new(),
        })
    }

    /// Test-only constructor: short timeouts so the timeout test doesn't
    /// take ten seconds, routed through the SAME `build` that assembles the
    /// User-Agent — no second place constructs it. The default three
    /// gateways are unused by every test that calls this (they exercise
    /// `fetch_http` directly, never `fetch`/`fetch_ipfs_chain`); tests that
    /// DO need to control gateways use [`Self::new_for_test_with_gateways`].
    ///
    /// The per-host cadence is ZERO here: every mechanics test would otherwise
    /// pay [`DEFAULT_HOST_DELAY`] per request for no reason, and the cadence has
    /// its own test ([`Self::new_for_test_with_host_delay`]) that sets a value
    /// it can actually measure.
    #[cfg(test)]
    pub(crate) fn new_for_test(connect_timeout: Duration, total_timeout: Duration) -> Self {
        Self::build(
            "https://agentcount.ai/methodology; contact: probes@agentcount.ai",
            &[
                "https://ipfs.io/ipfs/".to_string(),
                "https://cloudflare-ipfs.com/ipfs/".to_string(),
                "https://gateway.pinata.cloud/ipfs/".to_string(),
            ],
            connect_timeout,
            total_timeout,
            Duration::ZERO,
        )
        .expect("test client must build")
    }

    /// Test-only constructor for the per-host cadence and `Retry-After` tests,
    /// which are the only ones that need a non-zero delay — and need it small
    /// enough to measure without the test taking seconds.
    #[cfg(test)]
    pub(crate) fn new_for_test_with_host_delay(host_delay: Duration) -> Self {
        Self::build(
            "https://agentcount.ai/methodology; contact: probes@agentcount.ai",
            &["https://ipfs.io/ipfs/".to_string()],
            Duration::from_secs(2),
            Duration::from_secs(2),
            host_delay,
        )
        .expect("test client must build")
    }

    /// Test-only constructor for the P0 FIX 8 gateway-fallback tests: lets a
    /// test point the gateway chain at its own `wiremock` server URIs
    /// instead of the real, unreachable-from-tests production gateways.
    #[cfg(test)]
    pub(crate) fn new_for_test_with_gateways(
        ipfs_gateways: Vec<String>,
        connect_timeout: Duration,
        total_timeout: Duration,
    ) -> Self {
        Self::build(
            "https://agentcount.ai/methodology; contact: probes@agentcount.ai",
            &ipfs_gateways,
            connect_timeout,
            total_timeout,
            Duration::ZERO,
        )
        .expect("test client must build")
    }

    fn host_lane(&self, host: &str) -> Arc<HostLane> {
        let mut map = self.per_host.lock().expect("per_host mutex poisoned");
        map.entry(host.to_string())
            .or_insert_with(|| {
                Arc::new(HostLane {
                    permits: Semaphore::new(PER_HOST_CAP),
                    next_start: Mutex::new(Instant::now()),
                })
            })
            .clone()
    }

    /// Fetch one agent's declared URI. Infallible by design: every failure
    /// mode is a recorded [`FetchOutcome`], never an `Err` that would abort a
    /// 60,000-agent sweep.
    ///
    /// `ipfs://` is checked FIRST, ahead of [`resolve::resolve`] — P0 FIX 8
    /// means picking a gateway takes live network attempts (up to three, in
    /// sequence), which `resolve` deliberately never makes; see
    /// [`Self::fetch_ipfs_chain`].
    pub async fn fetch(&self, uri: &str) -> FetchOutcome {
        if let Some(cid_and_path) = resolve::ipfs_cid_and_path(uri) {
            return self.fetch_ipfs_chain(&cid_and_path, true).await;
        }
        match resolve::resolve(uri) {
            Target::Empty => FetchOutcome::base(""),
            Target::Unsupported { scheme } => FetchOutcome::base(scheme),
            Target::UnsupportedCompression { scheme, algorithm } => {
                // P0 FIX 7: we understood the declared codec, we simply
                // can't decode it — OUR limitation, recorded as `error` so
                // downstream reads `checks::CheckStatus::Error`, never
                // `Fail`.
                let mut out = FetchOutcome::base(scheme);
                out.error = Some(format!("unsupported_compression: {algorithm}"));
                out.elapsed_ms = Some(0);
                out
            }
            Target::Inline { bytes, decode } => Self::inline_outcome(bytes, decode),
            Target::Http { url } => self.fetch_http(url, None, true).await,
        }
    }

    fn inline_outcome(bytes: Vec<u8>, decode: DataUriDecode) -> FetchOutcome {
        let mut out = FetchOutcome::base("data");
        let (kept, truncated) = cap_bytes(bytes);
        out.body_sha256 = Some(sha256_hex(&kept));
        out.truncated = truncated;
        out.body = Some(kept);
        out.elapsed_ms = Some(0);
        out.inline_decode = Some(decode);
        out
    }

    /// P0 FIX 8: try each configured IPFS gateway in sequence until one
    /// answers with a 2xx, or all are exhausted. Each attempt goes through
    /// the FULL guarded path (`fetch_http`: netguard, robots, redirects,
    /// the body cap) exactly like any other HTTP fetch — the per-host
    /// concurrency cap in particular applies per GATEWAY HOST automatically,
    /// because `guarded_send`'s semaphore is keyed by `url.host_str()` and
    /// the three gateways are three different hosts; nothing here bypasses
    /// it. `validate_hops` is threaded straight through to `fetch_http` for
    /// every attempt — `false` only from this file's own mechanics tests
    /// (wiremock binds loopback, which the netguard rightly refuses).
    pub(crate) async fn fetch_ipfs_chain(
        &self,
        cid_and_path: &str,
        validate_hops: bool,
    ) -> FetchOutcome {
        let start = Instant::now();
        let mut attempts: Vec<GatewayAttempt> = Vec::new();
        let mut first_request_url: Option<String> = None;

        for gateway in &self.ipfs_gateways {
            let rewritten = format!("{gateway}{cid_and_path}");
            let url = match url::Url::parse(&rewritten) {
                Ok(u) => u,
                Err(e) => {
                    attempts.push(GatewayAttempt {
                        gateway: gateway.clone(),
                        http_status: None,
                        error: Some(format!("bad_gateway_url: {e}")),
                    });
                    continue;
                }
            };
            if first_request_url.is_none() {
                first_request_url = Some(url.to_string());
            }

            let attempt = self
                .fetch_http(url, Some(gateway.clone()), validate_hops)
                .await;
            let succeeded = matches!(attempt.http_status, Some(s) if (200..300).contains(&s));
            attempts.push(GatewayAttempt {
                gateway: gateway.clone(),
                http_status: attempt.http_status,
                error: attempt.error.clone(),
            });
            if succeeded {
                let mut out = attempt;
                out.gateway_attempts = attempts;
                // Total wall-clock across every attempt in the chain, not
                // just the winning one — the honest cost of this agent's
                // ipfs:// fetch.
                out.elapsed_ms = Some(elapsed_ms(start));
                return out;
            }
        }

        // Every gateway was tried and none answered 2xx. We genuinely
        // cannot tell an unpinned CID from a network problem on our end —
        // `error`, never `fail`. See P0 FIX 8.
        let mut out = FetchOutcome::base("ipfs");
        out.request_url = first_request_url;
        out.error = Some("ipfs_all_gateways_failed".into());
        out.gateway_attempts = attempts;
        out.elapsed_ms = Some(elapsed_ms(start));
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
                match self
                    .check_robots(&current, current.path(), validate_hops)
                    .await
                {
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
            out.retry_after_secs = resp.retry_after_secs;
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

    /// Re-validate one hop (initial URL or a redirect target) against the
    /// netguard's SSRF check. `pub(crate)` so `robots.rs` can apply the exact
    /// same guard to robots.txt's own redirect hops — a redirect there is
    /// just as attacker-controlled as a redirect on the main document.
    ///
    /// `url` here is always an already-parsed `http(s)://` URL — the
    /// initial request or a `Location` target — never a raw `ipfs://`
    /// string, so `netguard::resolve`'s own `ipfs://`-rewriting branch is
    /// unreachable through this call; the empty string below is never used.
    pub(crate) async fn validate_hop(&self, url: &url::Url) -> Result<(), String> {
        match netguard::resolve(url.as_str(), "").await {
            netguard::Resolution::Fetch(_) => Ok(()),
            netguard::Resolution::Reject(reason) => Err(reason),
            netguard::Resolution::Inline(_) => {
                Err("unexpected inline resolution while validating an http(s) hop".into())
            }
        }
    }

    /// Send one GET, gated by the per-host and global semaphores (held for
    /// the whole exchange, body read included — the connection is still
    /// occupying the host's attention while we stream a capped body) and by
    /// the host's cadence, and bounded by `total_timeout` regardless of how
    /// `reqwest` internally scopes its own timeout.
    ///
    /// Order matters: the host permit, then the host's next-start slot, then
    /// the global permit. Claiming the cadence slot before the global permit
    /// would hold a global permit while sleeping, which would let one
    /// rate-limited host throttle the whole sweep instead of only itself.
    pub(crate) async fn guarded_send(&self, url: &url::Url) -> Result<RawResponse, SendError> {
        let host = url.host_str().unwrap_or_default().to_string();
        let lane = self.host_lane(&host);
        let _host_permit = lane
            .permits
            .acquire()
            .await
            .expect("semaphore never closes");

        // Claim this request's place in the host's cadence, then wait for it.
        // The lock is released before the sleep — every waiting task takes a
        // distinct, already-reserved slot rather than racing for the same one.
        let wait = lane.claim_next_start(self.host_delay);
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }

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
            // Only 429 and 503 define this header's meaning; reading it off
            // anything else would be inventing a request the origin did not
            // make. See `parse_retry_after`.
            let retry_after_secs = if matches!(status, 429 | 503) {
                headers
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| parse_retry_after(v, chrono::Utc::now()))
            } else {
                None
            };
            let (body, truncated) = read_body_capped(resp)
                .await
                .map_err(|e| SendError::Connection(e.to_string()))?;
            Ok(RawResponse {
                status,
                headers,
                body,
                truncated,
                location,
                retry_after_secs,
            })
        };

        let result = match tokio::time::timeout(self.total_timeout, attempt).await {
            Ok(result) => result,
            Err(_) => Err(SendError::Timeout),
        };

        // Honour the backoff before returning, so it is in force for every
        // request still queued behind this one on the same host — including
        // the ones already waiting on the semaphore above.
        if let Ok(resp) = &result
            && let Some(secs) = resp.retry_after_secs
        {
            lane.back_off(Duration::from_secs(u64::from(secs)).min(MAX_HONOURED_RETRY_AFTER));
        }
        result
    }
}

impl HostLane {
    /// Reserve the next moment a request may start on this host, and return
    /// how long the caller must wait for it.
    ///
    /// `delay` of zero means no cadence: the slot is always now.
    fn claim_next_start(&self, delay: Duration) -> Duration {
        let now = Instant::now();
        let mut next = self.next_start.lock().expect("host lane mutex poisoned");
        let start_at = (*next).max(now);
        *next = start_at + delay;
        start_at.saturating_duration_since(now)
    }

    /// Push this host's next start at least `backoff` into the future.
    ///
    /// `max`, not `=`: a longer wait already claimed by the cadence (or by an
    /// earlier, larger `Retry-After`) must not be shortened by this one.
    fn back_off(&self, backoff: Duration) {
        let until = Instant::now() + backoff;
        let mut next = self.next_start.lock().expect("host lane mutex poisoned");
        if until > *next {
            *next = until;
        }
    }
}

/// Parse a `Retry-After` value into whole seconds from `now`.
///
/// RFC 9110 §10.2.3 allows either delta-seconds or an HTTP-date, and real rate
/// limiters send both. A date in the past is `Some(0)` — the origin named a
/// moment that has already arrived, which is a different fact from having sent
/// no header at all, and the caller records the difference.
///
/// Anything unparseable is `None`: guessing at a malformed backoff would be
/// worse than falling back to the ordinary cadence.
fn parse_retry_after(value: &str, now: chrono::DateTime<chrono::Utc>) -> Option<u32> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<u32>() {
        return Some(secs);
    }
    let when = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let secs = (when.with_timezone(&chrono::Utc) - now).num_seconds();
    Some(secs.clamp(0, i64::from(u32::MAX)) as u32)
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

    use base64::Engine;
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

    // --- politeness: rate, not just concurrency (2026-08-06) --------------

    /// The cap the test above measures bounds SIMULTANEITY. This one measures
    /// the thing that actually generated 19,658 HTTP 429s from one host: a
    /// fast-answering server can be asked again the instant we let go, so six
    /// requests to one host must take at least five cadence intervals however
    /// quickly it answers.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn requests_to_one_host_are_spaced_out_however_fast_it_answers() {
        let server = MockServer::start().await;
        allow_robots(&server).await;
        Mock::given(method("GET"))
            .and(path("/card.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let delay = Duration::from_millis(80);
        let prober = Arc::new(Prober::new_for_test_with_host_delay(delay));
        let url = target_url(&server, "/card.json");

        let started = Instant::now();
        let mut handles = Vec::new();
        for _ in 0..6 {
            let prober = prober.clone();
            let url = url.clone();
            handles.push(tokio::spawn(async move {
                prober.fetch_http(url, None, false).await
            }));
        }
        for h in handles {
            assert_eq!(h.await.unwrap().http_status, Some(200));
        }
        let elapsed = started.elapsed();

        // Six document fetches plus the one robots.txt fetch share the lane,
        // so there are at least six intervals between the first start and the
        // last. Asserting five is the same claim with room for the scheduler.
        assert!(
            elapsed >= delay * 5,
            "six requests to one host finished in {elapsed:?}, which is less \
             than five {delay:?} intervals — the cadence is not being applied"
        );
    }

    /// The cadence is per host. If it were global it would be a sweep-wide
    /// rate limit — a different, far more expensive promise — and a busy host
    /// would slow down every stranger it has nothing to do with.
    ///
    /// Exercised against the lane map directly rather than through two mock
    /// servers, because `wiremock` binds every server on `127.0.0.1` and the
    /// lane is keyed by HOST: two mock servers are two ports on one host, and
    /// they correctly share a lane. That is also the production behaviour worth
    /// stating — `example.com:8443` and `example.com` are one machine's
    /// attention, and one lane.
    #[test]
    fn each_host_has_its_own_lane_and_a_busy_one_never_delays_another() {
        let prober = fast_prober();
        let delay = Duration::from_millis(200);

        let busy = prober.host_lane("busy.example");
        let quiet = prober.host_lane("quiet.example");
        assert!(
            !Arc::ptr_eq(&busy, &quiet),
            "two hosts must not share one lane"
        );

        // Spend four of the busy host's slots: the fourth waits three
        // intervals, and the cadence is measured from the claim, not the send.
        let waits: Vec<Duration> = (0..4).map(|_| busy.claim_next_start(delay)).collect();
        assert_eq!(waits[0], Duration::ZERO);
        // Each claim reads the clock again, so the returned wait is a hair
        // under the nominal interval — the slack is for that, not for slop.
        assert!(
            waits[3] + Duration::from_millis(5) >= delay * 3,
            "the fourth claim waits {:?}, not three {delay:?} intervals",
            waits[3]
        );

        // The quiet host is untouched by any of it.
        assert_eq!(quiet.claim_next_start(delay), Duration::ZERO);

        // And the same host asked for twice is the same lane, so a second
        // fetch cannot escape the first one's cadence by re-looking it up.
        assert!(Arc::ptr_eq(&busy, &prober.host_lane("busy.example")));
    }

    #[test]
    fn a_backoff_never_shortens_a_wait_already_claimed() {
        // `back_off` takes the later of the two instants. A one-second
        // Retry-After arriving while the lane is already reserved five seconds
        // out must not pull the lane forward — that would turn honouring a
        // backoff into ignoring the cadence.
        let prober = fast_prober();
        let lane = prober.host_lane("h.example");
        let claimed = lane.claim_next_start(Duration::from_secs(5));
        assert_eq!(claimed, Duration::ZERO);
        // The lane is now reserved 5s out.
        lane.back_off(Duration::from_millis(1));
        let next = lane.claim_next_start(Duration::ZERO);
        assert!(
            next >= Duration::from_secs(4),
            "a shorter backoff pulled the lane forward to {next:?}"
        );

        // A longer one does move it.
        lane.back_off(Duration::from_secs(30));
        assert!(lane.claim_next_start(Duration::ZERO) >= Duration::from_secs(29));
    }

    #[tokio::test]
    async fn a_429_with_retry_after_is_recorded_and_backs_the_host_off() {
        let server = MockServer::start().await;
        allow_robots(&server).await;
        Mock::given(method("GET"))
            .and(path("/rate-limited"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
            .mount(&server)
            .await;

        // A cadence of zero, so the ONLY thing that can delay the second
        // request is the Retry-After the first one received.
        let prober = Prober::new_for_test_with_host_delay(Duration::ZERO);
        let url = target_url(&server, "/rate-limited");

        let first = prober.fetch_http(url.clone(), None, false).await;
        assert_eq!(first.http_status, Some(429));
        assert_eq!(
            first.retry_after_secs,
            Some(1),
            "the backoff the origin asked for must be recorded, not just obeyed"
        );
        assert!(
            first.error.is_none(),
            "a 429 is an answer; `crates/checks` decides what it means"
        );

        let started = Instant::now();
        let second = prober.fetch_http(url, None, false).await;
        assert_eq!(second.http_status, Some(429));
        assert!(
            started.elapsed() >= Duration::from_millis(900),
            "the second request went out after {:?}, ignoring the one-second \
             backoff the host asked for",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn a_retry_after_on_a_status_it_is_not_defined_for_is_ignored() {
        // Reading it off a 404 would invent a request the origin never made.
        let server = MockServer::start().await;
        allow_robots(&server).await;
        Mock::given(method("GET"))
            .and(path("/gone"))
            .respond_with(ResponseTemplate::new(404).insert_header("Retry-After", "600"))
            .mount(&server)
            .await;

        let prober = fast_prober();
        let out = prober
            .fetch_http(target_url(&server, "/gone"), None, false)
            .await;
        assert_eq!(out.http_status, Some(404));
        assert!(out.retry_after_secs.is_none());
    }

    #[test]
    fn retry_after_parses_both_forms_and_refuses_to_guess() {
        let now = chrono::DateTime::parse_from_rfc2822("Thu, 06 Aug 2026 08:00:00 GMT")
            .unwrap()
            .with_timezone(&chrono::Utc);

        assert_eq!(parse_retry_after("120", now), Some(120));
        assert_eq!(parse_retry_after("  30  ", now), Some(30));
        assert_eq!(parse_retry_after("0", now), Some(0));

        // The HTTP-date form, which Cloudflare and friends send.
        assert_eq!(
            parse_retry_after("Thu, 06 Aug 2026 08:02:00 GMT", now),
            Some(120)
        );
        // A date already past is zero, not a negative wait and not "no header".
        assert_eq!(
            parse_retry_after("Thu, 06 Aug 2026 07:59:00 GMT", now),
            Some(0)
        );

        for junk in ["", "soon", "-5", "1.5", "next tuesday"] {
            assert_eq!(parse_retry_after(junk, now), None, "{junk:?}");
        }
    }

    #[test]
    fn an_absurd_retry_after_is_clamped_to_what_we_will_actually_wait() {
        // The clamp is what keeps one host from holding a share of the sweep,
        // and it must not silently become "ignore the header".
        let asked = Duration::from_secs(3600);
        assert_eq!(
            asked.min(MAX_HONOURED_RETRY_AFTER),
            MAX_HONOURED_RETRY_AFTER
        );
        assert!(MAX_HONOURED_RETRY_AFTER > DEFAULT_HOST_DELAY);
    }

    #[test]
    fn the_published_politeness_numbers_are_what_this_module_enforces() {
        // METHODOLOGY.md §6 states these; a test is the only thing that keeps
        // the document and the code from drifting apart.
        assert_eq!(PER_HOST_CAP, 2);
        assert_eq!(DEFAULT_HOST_DELAY, Duration::from_millis(250));
        assert_eq!(MAX_HONOURED_RETRY_AFTER, Duration::from_secs(120));
    }

    #[test]
    fn user_agent_matches_the_published_methodology_string() {
        // METHODOLOGY.md line 301 is the single source for this string;
        // Prober::new's contact_url parameter must reproduce it exactly.
        let contact_url = "https://agentcount.ai/methodology; contact: probes@agentcount.ai";
        let ua = format!("{PRODUCT_TOKEN}/{PRODUCT_VERSION} (+{contact_url})");
        assert_eq!(
            ua,
            "agentcount-probe/0.2 (+https://agentcount.ai/methodology; contact: probes@agentcount.ai)"
        );
    }

    // --- P0 FIX 7: an unsupported compression algorithm is `error` -------

    #[tokio::test]
    async fn an_unsupported_compression_algorithm_is_error_not_fail_and_touches_no_network() {
        // No mock server needed at all: Target::UnsupportedCompression is
        // decided entirely by `resolve()`, before any request would be made.
        let prober = fast_prober();
        let out = prober
            .fetch("data:application/json;enc=zstd;base64,eyJhIjoxfQ==")
            .await;

        assert_eq!(out.scheme, "data");
        assert_eq!(out.error.as_deref(), Some("unsupported_compression: zstd"));
        assert!(out.http_status.is_none());
        assert!(out.body.is_none());
    }

    #[tokio::test]
    async fn a_gzip_compressed_data_uri_is_decoded_via_the_public_fetch_entry_point() {
        use std::io::Write;
        let plaintext = br#"{"name":"gzip via fetch()"}"#;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
        encoder.write_all(plaintext).unwrap();
        let gz = encoder.finish().unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&gz);
        let uri = format!("data:application/json;enc=gzip;level=6;base64,{b64}");

        let prober = fast_prober();
        let out = prober.fetch(&uri).await;

        assert_eq!(out.scheme, "data");
        assert!(out.error.is_none());
        assert_eq!(out.body.as_deref(), Some(plaintext.as_slice()));
        let decode = out
            .inline_decode
            .expect("decode provenance must be recorded");
        assert_eq!(decode.variant, "compressed");
        assert_eq!(decode.algorithm.as_deref(), Some("gzip"));
    }

    // --- P0 FIX 8: IPFS gateway fallback chain ----------------------------

    async fn mock_gateway(status: u16, body: Option<&str>) -> MockServer {
        let server = MockServer::start().await;
        allow_robots(&server).await;
        let mut template = ResponseTemplate::new(status);
        if let Some(b) = body {
            template = template.set_body_string(b);
        }
        Mock::given(method("GET"))
            .and(path("/ipfs/bafyfakecid"))
            .respond_with(template)
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn an_ipfs_fetch_falls_back_to_the_second_gateway_when_the_first_fails() {
        let gw1 = mock_gateway(404, None).await;
        let gw2 = mock_gateway(200, Some("the card")).await;
        let gateways = vec![
            format!("{}/ipfs/", gw1.uri()),
            format!("{}/ipfs/", gw2.uri()),
        ];

        let prober = Prober::new_for_test_with_gateways(
            gateways,
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let out = prober.fetch_ipfs_chain("bafyfakecid", false).await;

        assert_eq!(out.http_status, Some(200));
        assert!(
            out.error.is_none(),
            "a gateway that eventually answers is not our error"
        );
        assert_eq!(out.body.as_deref(), Some(b"the card".as_slice()));
        assert_eq!(
            out.via_gateway.as_deref(),
            Some(format!("{}/ipfs/", gw2.uri())).as_deref()
        );

        // The whole chain is recorded, not just the winner.
        assert_eq!(out.gateway_attempts.len(), 2);
        assert_eq!(out.gateway_attempts[0].http_status, Some(404));
        assert_eq!(out.gateway_attempts[1].http_status, Some(200));
    }

    #[tokio::test]
    async fn ipfs_all_gateways_failing_is_error_never_fail() {
        // Every server must stay alive for the whole test — dropping a
        // `MockServer` triggers wiremock's graceful shutdown immediately,
        // freeing its port for potential reuse by another test running
        // concurrently. Collecting into a `Vec` (rather than a `let _ =` per
        // iteration) keeps all three bound until this function returns.
        let mut servers = Vec::new();
        let mut gateways = Vec::new();
        for _ in 0..3 {
            let gw = mock_gateway(404, None).await;
            gateways.push(format!("{}/ipfs/", gw.uri()));
            servers.push(gw);
        }

        let prober = Prober::new_for_test_with_gateways(
            gateways,
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let out = prober.fetch_ipfs_chain("bafyfakecid", false).await;

        // We cannot tell an unpinned CID from a network problem on our end —
        // this must never surface as a bare non-2xx status left to be
        // misjudged as the agent's `fail`.
        assert_eq!(out.error.as_deref(), Some("ipfs_all_gateways_failed"));
        assert!(out.http_status.is_none());
        assert_eq!(out.gateway_attempts.len(), 3);
        assert!(
            out.gateway_attempts
                .iter()
                .all(|a| a.http_status == Some(404)),
            "every attempt's own status must still be recorded: {:?}",
            out.gateway_attempts
        );
    }

    #[tokio::test]
    async fn the_first_gateway_succeeding_never_calls_the_others() {
        let gw1 = mock_gateway(200, Some("first answers")).await;
        // No mock configured on gw2/gw3 at all — the assertion on
        // `gateway_attempts.len()` below is what actually proves neither was
        // ever called, not the absence of a mock.
        let gw2 = MockServer::start().await;
        allow_robots(&gw2).await;
        let gw3 = MockServer::start().await;
        allow_robots(&gw3).await;
        let gateways = vec![
            format!("{}/ipfs/", gw1.uri()),
            format!("{}/ipfs/", gw2.uri()),
            format!("{}/ipfs/", gw3.uri()),
        ];

        let prober = Prober::new_for_test_with_gateways(
            gateways,
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let out = prober.fetch_ipfs_chain("bafyfakecid", false).await;

        assert_eq!(out.http_status, Some(200));
        assert_eq!(out.gateway_attempts.len(), 1);
    }
}
