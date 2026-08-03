//! The on-demand spot check: run the ladder for ONE agent, right now.
//!
//! `POST /api/agents/{chain}/{id}/spot-check` reads the agent off the chain at
//! the current head, fetches its declared document through the census's own
//! guarded prober, and answers the rungs that can be answered without a run.
//!
//! Everything in this file exists to keep two promises. The first is about
//! what the answer MEANS. The second is about what the endpoint DOES to other
//! people's servers. They are separate promises and they are kept separately.
//!
//! # Promise 1: a spot check is not a measurement of record
//!
//! The census's authority is the pin: *N agents as they existed simultaneously
//! at block X*. A spot check is one agent at whatever block the chain happened
//! to be at when somebody clicked a button. Those are different kinds of
//! claim, and blending them would destroy the first without improving the
//! second.
//!
//! So, structurally:
//!
//! * **It has no `run_id`, because it is not part of a run.** Nothing here
//!   opens, joins, or writes to `runs`, `agent_snapshots`, `check_results`,
//!   `http_archive` or `agent_documents`. The only database read is the
//!   `chains` row that says which registry to talk to.
//! * **Nothing is written. Anywhere.** See "Store or not store" below.
//! * **The response type is not the census's.** [`SpotCheckResponse`] is
//!   deliberately NOT `routes::agents::AgentDetail`: it carries `source:
//!   "spot_check"`, a `notice`, its own `checked_at` / `block_number` /
//!   `checker_version` / `checker_commit`, and names its rung list `checks`
//!   rather than `rungs`. A screenshot of one cannot be passed off as the
//!   other, and neither can a curl output — every top-level key differs
//!   somewhere.
//!
//! ## Store or not store: NOT STORED, and why
//!
//! There is no `spot_checks` table and no migration in this change. Three
//! reasons, in the order they mattered:
//!
//! 1. **A stored spot check is evidence of third-party probing, held by us.**
//!    A table of "who asked us to send a request to whose server, when" is
//!    exactly the artifact a subpoena asks for, and it is an artifact this
//!    product has no use for. The census's own probing is already documented,
//!    disclosed and defensible because it is a published, methodologically
//!    motivated sweep. A log of strangers' one-off probes is none of those
//!    things, and keeping it would mean acquiring a liability on behalf of
//!    people who only clicked a button.
//! 2. **It grows with demand, not with the population.** Every other table
//!    here is bounded by the number of agents on chain times the number of
//!    runs. This one would be bounded by traffic, which is to say unbounded,
//!    and its retention policy would be a thing somebody has to own forever.
//! 3. **A stored spot check invites aggregation, and any aggregate of it
//!    would be a lie.** The day the table exists is the day someone writes
//!    `SELECT status, count(*) FROM spot_checks GROUP BY status` and puts the
//!    result on a slide. That number is not a rate: its denominator is
//!    "agents somebody happened to click on", the most self-selected sample
//!    imaginable, biased toward agents whose owners are checking their own
//!    work. The safest way to prevent a wrong number from being computed is
//!    for the rows not to exist. (Had we stored them, the table would have
//!    had to be non-run-scoped for the structural reason migration 0018 gives
//!    for `registration_tail`: no `run_id` means no census query can reach it
//!    by forgetting a filter. Not storing is the same argument taken one step
//!    further — no row means no query at all.)
//!
//! **What that costs, stated plainly.** No audit trail: if this endpoint is
//! abused, the record is whatever the process logged and the platform's own
//! request log, not a table we can query. No cache: every call does the work
//! or does not happen. The cache is not actually a loss — a cached spot check
//! is a *stale* spot check wearing a fresh timestamp, which is precisely the
//! confusion this whole file is built to prevent. The audit trail is a real
//! cost, accepted; the rate limiter below is the control that replaces it,
//! and it is preventive rather than forensic on purpose.
//!
//! # Promise 2: this endpoint is not a probe amplifier
//!
//! Left open, `POST …/spot-check` is a way to make AgentCount's servers send
//! requests to somebody else's, on demand, from an address that is not the
//! requester's. Four controls, all of them necessary:
//!
//! * **Only real agents.** The path names a chain and an agent id, never a
//!   URL. The URL fetched is the one `tokenURI()` returned for an id the
//!   registry confirmed exists ([`chain::Registry::exists`], the same call the
//!   census's population walk uses). There is no arbitrary-URL probing
//!   endpoint here and there must never be one.
//! * **The census's own prober, unmodified.** [`probe::Prober`] brings the
//!   identifying User-Agent with a contact address, robots.txt (honored,
//!   including its redirects), the per-host concurrency cap, connect and total
//!   timeouts, the redirect cap, the response-size cap, and the SSRF netguard
//!   re-run on every hop. Reimplementing any of that here would have produced
//!   a second, less careful HTTP client pointed at strangers.
//! * **Two rate limits** (see [`PER_CLIENT_QUOTAS`] / [`PER_HOST_QUOTAS`]),
//!   one keyed on the caller and one on the target host. The second is the one
//!   that matters; the first is a courtesy.
//! * **Rung 6 is not probed at all** (see [`NOT_CHECKED_RUNG6`]).
//!
//! ## Why `POST` and not `GET`
//!
//! A spot check is not a read. It sends real traffic to a real third party, so
//! it must not be reachable by anything that follows links speculatively:
//! browser prefetch, link-preview unfurlers, crawlers, `<img>` and `<script>`
//! src, or a shared URL in a chat window. All of those issue `GET`. None of
//! them issues `POST`. Idempotence is beside the point — the result of a spot
//! check is *supposed* to change, that is what "right now" means — and treating
//! it as a cacheable read is exactly the mistake that turns a hyperlink into a
//! probe trigger. `POST` also forces a CORS preflight for cross-origin
//! callers, so a hostile page cannot fire one invisibly from a visitor's
//! browser.
//!
//! # The checks are the census's checks
//!
//! Every verdict comes from `crates/checks`, through the same functions the
//! sweeper calls, and the gating between them comes from `checks::run_ladder`
//! and nowhere else. The scheme bucket rung 2 is judged against comes from
//! `probe::FetchOutcome::scheme_bucket`, which the sweeper also calls — it was
//! moved into `crates/probe` by this change for exactly that reason. A spot
//! check and a census row can disagree about the WORLD (a server that was up
//! in July may be down today; that is the point of the feature) but they can
//! never disagree about what a status MEANS.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::AppState;
use crate::error::{ApiError, ApiResult};

// ─────────────────────────────────────────────────────────────────────────────
// Rate limiting
// ─────────────────────────────────────────────────────────────────────────────

/// One "at most `limit` in any `window`" rule.
#[derive(Debug, Clone, Copy)]
pub struct Quota {
    pub limit: usize,
    pub window: Duration,
}

/// **Per calling client.** Five spot checks per ten minutes.
///
/// Sized for the actual human use: somebody on an agent page clicks "check
/// now", maybe compares two or three agents, maybe re-checks one after fixing
/// their document. Five in ten minutes covers all of that with room to spare.
///
/// It is also enough to make the endpoint useless as a scanner. At five per
/// ten minutes, walking the 354,858 agents in the published population from
/// one address takes about 1.35 years. Anyone who wants the whole population's
/// answers already has them: the census publishes them, in bulk, for free.
///
/// This limit is **best-effort and deliberately not load-bearing** — see
/// [`client_key`] for why the key it is applied to is only as trustworthy as
/// the proxy chain in front of this process. The limit that actually protects
/// a third party is the next one.
pub const PER_CLIENT_QUOTAS: &[Quota] = &[Quota {
    limit: 5,
    window: Duration::from_secs(600),
}];

/// **Per target host.** One spot check per minute, and at most twenty per hour.
///
/// This is the control the endpoint's safety rests on, because it is keyed on
/// something the caller cannot choose freely: the host of the URI the agent
/// itself published on chain. A botnet of ten thousand addresses defeats
/// [`PER_CLIENT_QUOTAS`] completely and does not move this one at all — the
/// victim host still receives one request per minute, from us, in total.
///
/// It has to be keyed on the host rather than the agent, because the obvious
/// attack is to aim many *different* agent ids at one server. That is not
/// hypothetical: four hosts carry 59.2% of all declared service endpoints in
/// the census, so for the largest operators there are tens of thousands of
/// distinct, real, registry-confirmed agent ids all pointing at the same
/// machine. Per-agent limiting would have been theatre.
///
/// The numbers, against what the census itself already does: a sweep sends a
/// host one document request per agent per run, with at most
/// `probe::PER_HOST_CAP` (2) in flight, roughly monthly. One per minute is
/// below the rate a single ordinary visitor to that host generates, and it is
/// a rate no operator can distinguish from background noise. The hourly cap
/// exists so a sustained campaign cannot turn "one per minute" into 1,440
/// requests a day: twenty per hour caps a determined attacker at 480, which
/// is under a single day's census budget for that host and spread across the
/// whole day rather than delivered in a burst.
///
/// **Known limit of the implementation:** the counters are in-process, so N
/// API instances allow N times these rates. That is stated rather than hidden;
/// the service runs with a small instance ceiling, and the global
/// `ConcurrencyLimitLayer` bounds the rest. A shared counter (Redis, or a
/// Postgres table) would fix it and would also reintroduce exactly the stored,
/// subpoena-able record of third-party probing that "Store or not store"
/// above rejects. Given the choice between a limit that is 3x looser than
/// stated and a permanent log of who probed whom, this takes the looser limit.
pub const PER_HOST_QUOTAS: &[Quota] = &[
    Quota {
        limit: 1,
        window: Duration::from_secs(60),
    },
    Quota {
        limit: 20,
        window: Duration::from_secs(3600),
    },
];

/// How many distinct keys a limiter tracks before it sweeps expired ones.
///
/// Without this, the key map is a memory leak with a stranger's hand on the
/// tap: every new client address adds an entry that nothing ever removes. The
/// sweep drops any key whose newest hit is older than the longest window,
/// which is precisely a key whose budget is fully restored — so eviction can
/// never lose a limit that was still in force.
const MAX_TRACKED_KEYS: usize = 20_000;

/// A sliding-window counter, keyed by an arbitrary string, evaluated against
/// several [`Quota`]s at once.
///
/// The clock is a parameter, not a call to [`Instant::now`], so every rule
/// below is testable without sleeping — see this module's tests.
pub struct RateLimiter {
    /// Named so a 429 can say which limit bound without leaking counts.
    what: &'static str,
    quotas: &'static [Quota],
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new(what: &'static str, quotas: &'static [Quota]) -> Self {
        assert!(
            !quotas.is_empty(),
            "a limiter with no quotas limits nothing"
        );
        Self {
            what,
            quotas,
            hits: Mutex::new(HashMap::new()),
        }
    }

    fn longest_window(&self) -> Duration {
        self.quotas
            .iter()
            .map(|q| q.window)
            .max()
            .expect("constructor asserts at least one quota")
    }

    /// Take one unit of budget for `key`, or say how long to wait.
    ///
    /// `Ok(())` records the hit. `Err(d)` records nothing — a caller who is
    /// being refused must not have their refusal push their own next attempt
    /// further away, which is what counting rejected attempts would do.
    pub fn try_acquire(&self, key: &str, now: Instant) -> Result<(), Duration> {
        let longest = self.longest_window();
        let mut hits = self.hits.lock().expect("rate limiter mutex poisoned");

        if hits.len() > MAX_TRACKED_KEYS {
            hits.retain(|_, times| {
                times
                    .back()
                    .is_some_and(|t| now.duration_since(*t) < longest)
            });
        }

        let times = hits.entry(key.to_string()).or_default();
        while times
            .front()
            .is_some_and(|t| now.duration_since(*t) >= longest)
        {
            times.pop_front();
        }

        let mut wait: Option<Duration> = None;
        for quota in self.quotas {
            let in_window: Vec<Instant> = times
                .iter()
                .copied()
                .filter(|t| now.duration_since(*t) < quota.window)
                .collect();
            if in_window.len() < quota.limit {
                continue;
            }
            // `in_window` is oldest-first. To get back under `limit` we need
            // `len - limit + 1` of the oldest hits to fall out of the window,
            // so the one to wait for is at index `len - limit`.
            let blocker = in_window[in_window.len() - quota.limit];
            let elapsed = now.duration_since(blocker);
            let remaining = quota.window.saturating_sub(elapsed);
            wait = Some(wait.map_or(remaining, |w: Duration| w.max(remaining)));
        }

        match wait {
            Some(d) => Err(d),
            None => {
                times.push_back(now);
                Ok(())
            }
        }
    }

    /// The 429 this limiter produces. `Retry-After` is in whole seconds and
    /// never zero: a sub-second remainder rounds up, because a `Retry-After: 0`
    /// tells a client to retry immediately, which is not what any limit here
    /// means.
    fn refuse(&self, wait: Duration) -> ApiError {
        let secs = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
        ApiError::TooManyRequests {
            retry_after_secs: secs.max(1),
            message: format!(
                "spot checks are rate limited per {} — retry in {} seconds. \
                 The full census results for this agent are available without a \
                 limit at GET /api/agents/{{chain}}/{{id}}.",
                self.what,
                secs.max(1)
            ),
        }
    }
}

/// Which entry of `X-Forwarded-For` names the caller, counted from the right.
///
/// `1` (the default) means the last entry: the one appended by the hop closest
/// to this process. Every entry to its left was written by something further
/// away and is fully caller-controlled, so a caller who sends their own
/// `X-Forwarded-For:` header only ever *prepends* noise.
///
/// Behind a Google Cloud load balancer the chain ends `…, <client>, <lb>`, so
/// the caller is the second entry from the right: set
/// `SPOT_CHECK_XFF_DEPTH=2` there. Getting this wrong is not a security hole
/// in either direction — too small a depth puts every caller in one shared
/// bucket (over-limiting, annoying, safe), too large a depth lets a caller
/// pick their own bucket (under-limiting the per-CLIENT quota only, never the
/// per-HOST one, which is the limit that protects anybody but us).
fn xff_depth() -> usize {
    std::env::var("SPOT_CHECK_XFF_DEPTH")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(1)
}

/// The key [`PER_CLIENT_QUOTAS`] is applied to.
///
/// Pure so the header-parsing rules are testable. `peer` is the TCP peer,
/// which is the truth when nothing is proxying and the load balancer's own
/// address when something is — hence the header preference.
///
/// Reads the chain from the right, the same rule `routes::subscribe`'s
/// `client_ip` already uses for the same platform and the same reason.
/// Generalised here only by making the depth configurable, because this
/// limiter guards outbound traffic rather than a newsletter row.
pub(crate) fn client_key(headers: &HeaderMap, peer: Option<SocketAddr>, depth: usize) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let entries: Vec<&str> = xff
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if !entries.is_empty() {
            // Saturating: a chain shorter than `depth` yields its leftmost
            // entry rather than falling through to the peer address, so a
            // misconfigured depth degrades to over-limiting, never to
            // "everyone gets their own bucket".
            let index = entries.len().saturating_sub(depth);
            return entries[index.min(entries.len() - 1)].to_string();
        }
    }
    match peer {
        Some(addr) => addr.ip().to_string(),
        // One shared bucket. Refusing outright would be defensible; sharing a
        // bucket is the same protection for third parties without breaking a
        // deployment whose proxy setup we guessed wrong.
        None => "unknown".to_string(),
    }
}

/// The key [`PER_HOST_QUOTAS`] is applied to: the host this spot check is
/// about to send a request to, or `None` when it will send none.
///
/// `None` for an empty URI, a `data:` payload (decoded in-process, no
/// network), and an unsupported or unparseable scheme — the ~65% of on-chain
/// URIs that are garbage never touch a third party, so limiting them would
/// only stop people from learning that they are garbage.
///
/// For `ipfs://` the host is the first configured gateway. The gateway is a
/// third party too and gets the same protection. The prober may fall back to
/// two further gateways when the first does not answer; those are bounded by
/// `probe`'s own per-host cap and are infrastructure we chose deliberately,
/// not a server someone put on chain to point us at.
pub(crate) fn target_host(uri: &str, ipfs_gateways: &[String]) -> Option<String> {
    if probe::ipfs_cid_and_path(uri).is_some() {
        return ipfs_gateways
            .first()
            .and_then(|g| url::Url::parse(g).ok())
            .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()));
    }
    match probe::resolve(uri) {
        probe::Target::Http { url } => url.host_str().map(|h| h.to_ascii_lowercase()),
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Request parsing
// ─────────────────────────────────────────────────────────────────────────────

/// Agent ids are `uint256` token ids on chain but `u64` everywhere in this
/// codebase (see `chain::Registry`), so the parse is also the range check.
/// Taken as a string from the path rather than as a typed `u64` so that `-1`
/// and `999999999999999999999` produce a 400 that names the value, instead of
/// axum's generic path-rejection message.
pub(crate) fn parse_agent_id(raw: &str) -> ApiResult<u64> {
    raw.trim().parse::<u64>().map_err(|_| {
        ApiError::BadRequest(format!(
            "'{raw}' is not a valid agent id — it must be a whole number from 0 to {}",
            u64::MAX
        ))
    })
}

/// Chain names are lowercase identifiers (`base`, `ethereum`, …). Normalised
/// here and validated by shape before it ever reaches a query, so a chain
/// segment cannot smuggle anything into the `chains` lookup or into a log line.
pub(crate) fn normalise_chain(raw: &str) -> ApiResult<String> {
    let chain = raw.trim().to_ascii_lowercase();
    let ok = !chain.is_empty()
        && chain.len() <= 32
        && chain
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok {
        Ok(chain)
    } else {
        Err(ApiError::BadRequest(
            "chain must be a short lowercase identifier, e.g. 'base'".into(),
        ))
    }
}

/// What a registry read means for "does this agent exist".
///
/// Two things have to be true, and they fail differently:
///
/// * `ownerOf` must not revert. A revert is the registry saying "no such
///   token"; [`chain::Registry::exists`] already turns that into `false`, and
///   any OTHER RPC failure into an `Err`, so a flaky node can never be read as
///   an agent that is not there.
/// * The owner must not be the zero address. Some registries return it instead
///   of reverting for a burned token. That is rung 1's own rule
///   (`checks::registered`), and this restates the consequence for the probe
///   decision rather than the verdict: an id nobody holds is not an id we will
///   send a request on behalf of.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Existence {
    Registered,
    /// In the registry, but held by nobody. Rung 1 fails; nothing is fetched.
    Unheld,
}

const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";

pub(crate) fn existence_from_owner(owner: &str) -> Existence {
    if owner.trim().eq_ignore_ascii_case(ZERO_ADDRESS) {
        Existence::Unheld
    } else {
        Existence::Registered
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The response — deliberately not the census's shape
// ─────────────────────────────────────────────────────────────────────────────

/// The discriminator. Present on every response this endpoint produces, and on
/// no other endpoint's.
pub const SOURCE: &str = "spot_check";

/// Carried in the body so that a screenshot, a paste, or a curl transcript
/// still says what it is when the URL is no longer visible.
pub const NOTICE: &str = "This is an on-demand spot check of one agent at the block named below. \
     It is NOT a census measurement: it belongs to no run, it is not stored, \
     and it never enters a published rate, finding or archive. Census results \
     come from GET /api/agents/{chain}/{id}, which names its run.";

/// Rung 6's absence, worded once.
pub const NOT_CHECKED_RUNG6: &str = "not probed on demand: rung 6 sends one request per DECLARED endpoint, and a \
     document may declare any number of them, at any hosts its author chooses — \
     so a single spot check would become an unbounded burst aimed at a target the \
     caller picks. The census probes them under a per-host budget of 500 distinct \
     URLs per run with a deterministic sample, which is a discipline no one-off \
     check can reproduce. See the run's own rung 6 for this agent.";

/// Absence of rung 7 when rung 1 did not pass.
pub const NOT_CHECKED_RUNG7: &str = "rung 1 did not pass, so no reputation read was made — the ladder would \
     have marked it skipped, and asking would have cost a read whose answer is \
     discarded";

#[derive(Debug, Serialize)]
pub struct SpotIdentity {
    pub chain_id: u64,
    /// Identity Registry address, lowercase hex.
    pub registry: String,
    /// ERC-721 token id as a decimal STRING — `uint256` does not fit `i64` and
    /// a JSON number would lose precision above 2^53.
    pub token_id: String,
    pub owner: String,
    /// `tokenURI()`, verbatim. An empty string is a legitimate on-chain value.
    pub agent_uri: String,
}

/// What the one document fetch looked like. Never the body itself.
///
/// Deliberately NOT named `archive`: nothing was archived. These bytes were
/// read, judged and dropped.
#[derive(Debug, Serialize)]
pub struct SpotFetch {
    pub scheme: String,
    /// `None` when no network request was made at all — an empty URI, a
    /// `data:` payload decoded in process, or a scheme we do not fetch.
    pub request_url: Option<String>,
    pub final_url: Option<String>,
    pub http_status: Option<u16>,
    pub content_type: Option<String>,
    pub body_bytes: Option<usize>,
    pub body_sha256: Option<String>,
    pub truncated: bool,
    /// Plain text, never a verdict: `ssrf_blocked: …`, `robots_disallowed`,
    /// `timeout`, `connection_failed: …`. `crates/checks` is the only thing
    /// that turns one of these into a status.
    pub error: Option<String>,
    pub elapsed_ms: Option<u32>,
    pub via_gateway: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SpotRung {
    pub rung: u8,
    pub name: &'static str,
    pub status: String,
    pub evidence: serde_json::Value,
    pub checked_at: DateTime<Utc>,
}

/// A rung this check did not ask, and why. The absence itself is the claim —
/// this list explains it rather than replacing it, so a reader who only looks
/// at `checks` still sees "not asked" as a missing entry, exactly as they
/// would for a census run.
#[derive(Debug, Serialize)]
pub struct NotChecked {
    pub rung: u8,
    pub name: &'static str,
    pub reason: &'static str,
}

#[derive(Debug, Serialize)]
pub struct SpotCheckResponse {
    /// Always [`SOURCE`]. The discriminator.
    pub source: &'static str,
    pub notice: &'static str,
    pub chain: String,
    pub agent_id: u64,
    /// When this check ran. Not a run's `started_at` — there is no run.
    pub checked_at: DateTime<Utc>,
    /// The block every chain read here was pinned to: the head at the moment
    /// the button was pressed, not a census pin.
    pub block_number: u64,
    pub checker_version: &'static str,
    pub checker_commit: &'static str,
    pub schema_version: i32,
    pub spec_commit: &'static str,
    pub identity: SpotIdentity,
    /// `None` when no fetch was attempted, which happens only when rung 1 did
    /// not pass and the ladder would have discarded the answer anyway.
    pub fetch: Option<SpotFetch>,
    /// Every rung this check actually asked, in rung order. Named `checks`,
    /// not `rungs`, so the two response shapes cannot be confused field by
    /// field.
    pub checks: Vec<SpotRung>,
    pub not_checked: Vec<NotChecked>,
}

/// Everything [`shape`] needs, already reduced to primitives — so the shaping
/// rules are testable without an RPC connection or an HTTP client.
pub(crate) struct ShapeInput {
    pub chain: String,
    pub agent_id: u64,
    pub block_number: u64,
    pub chain_id: u64,
    pub registry: String,
    pub token_id: String,
    pub owner: String,
    pub agent_uri: String,
    pub fetch: Option<SpotFetch>,
    /// The ladder's output, already through `checks::run_ladder`.
    pub results: Vec<checks::CheckResult>,
    pub checked_at: DateTime<Utc>,
}

/// The seven rungs' names, so an absent one can be named in `not_checked`
/// without inventing a label. Index is `rung - 1`.
const RUNG_NAMES: [&str; 7] = [
    "registered",
    "resolvable",
    "parseable",
    "conformant",
    "bound",
    "live",
    "attested",
];

fn rung_name(rung: u8) -> &'static str {
    RUNG_NAMES
        .get(usize::from(rung).saturating_sub(1))
        .copied()
        .unwrap_or("unknown")
}

/// Turn the ladder's output into the wire shape, and account for every rung
/// that is not in it.
///
/// The accounting is a diff rather than a hardcoded list, so a rung that stops
/// being asked (or starts) cannot silently vanish from the response: anything
/// missing from `results` gets a `not_checked` entry, with the specific reason
/// when there is one and a generic one otherwise.
pub(crate) fn shape(input: ShapeInput) -> SpotCheckResponse {
    let mut results = input.results;
    results.sort_by_key(|r| r.rung);

    let asked: Vec<u8> = results.iter().map(|r| r.rung).collect();
    let not_checked = (1u8..=7)
        .filter(|r| !asked.contains(r))
        .map(|rung| NotChecked {
            rung,
            name: rung_name(rung),
            reason: match rung {
                6 => NOT_CHECKED_RUNG6,
                7 => NOT_CHECKED_RUNG7,
                _ => "not asked by this spot check",
            },
        })
        .collect();

    SpotCheckResponse {
        source: SOURCE,
        notice: NOTICE,
        chain: input.chain,
        agent_id: input.agent_id,
        checked_at: input.checked_at,
        block_number: input.block_number,
        checker_version: checks::CHECKER_VERSION,
        checker_commit: env!("CHECKER_COMMIT"),
        schema_version: checks::SCHEMA_VERSION,
        spec_commit: checks::SPEC_COMMIT,
        identity: SpotIdentity {
            chain_id: input.chain_id,
            registry: input.registry,
            token_id: input.token_id,
            owner: input.owner,
            agent_uri: input.agent_uri,
        },
        fetch: input.fetch,
        checks: results
            .into_iter()
            .map(|r| SpotRung {
                rung: r.rung,
                name: r.name,
                status: r.status.as_str().to_string(),
                evidence: r.evidence,
                checked_at: r.checked_at,
            })
            .collect(),
        not_checked,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The service: shared prober, per-chain clients, the two limiters
// ─────────────────────────────────────────────────────────────────────────────

/// The published contact string from `METHODOLOGY.md` §6.
///
/// Duplicated from `crates/sweeper/src/main.rs`'s constant of the same name
/// rather than shared, for the reason `probe::Prober::new` documents: the
/// crate that actually sends the header must never hardcode it, so each
/// caller states what it is identifying itself as. The two must stay
/// identical — a host that has allowlisted or blocked the census's probe must
/// see the same product token and mailbox here.
const PROBE_CONTACT_URL: &str = "https://agentcount.ai/methodology; contact: probes@agentcount.ai";

/// Same defaults and same env var as the sweeper's `ipfs_gateways()`, so a
/// spot check of an `ipfs://` agent tries the gateways a run would have.
fn ipfs_gateways() -> Vec<String> {
    std::env::var("IPFS_GATEWAYS")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|g| g.trim().to_string())
                .filter(|g| !g.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| {
            vec![
                "https://ipfs.io/ipfs/".to_string(),
                "https://cloudflare-ipfs.com/ipfs/".to_string(),
                "https://gateway.pinata.cloud/ipfs/".to_string(),
            ]
        })
}

/// The chain clients for one chain, connected once and reused.
struct ChainClients {
    registry: chain::Registry,
    /// `None` when `chains.reputation_registry` is NULL for this chain — which
    /// rung 7 reports as `error` (our limitation), never `fail`.
    reputation: Option<chain::Reputation>,
    chain_id: u64,
    identity_registry: String,
}

/// Everything the spot check needs that outlives a request.
pub struct SpotCheckService {
    prober: probe::Prober,
    ipfs_gateways: Vec<String>,
    per_client: RateLimiter,
    per_host: RateLimiter,
    /// Lazily connected, keyed by chain name. A `tokio` mutex because the
    /// connect it guards is `async`.
    chains: tokio::sync::Mutex<HashMap<String, std::sync::Arc<ChainClients>>>,
}

impl SpotCheckService {
    pub fn new() -> anyhow::Result<Self> {
        let gateways = ipfs_gateways();
        Ok(Self {
            // The census's prober, with the census's User-Agent, robots
            // handling, per-host cap, timeouts, redirect cap and body cap. One
            // instance for the process, so the per-host semaphore and the
            // robots cache are shared across every spot check — a second
            // instance would have meant a second, unbudgeted set of permits
            // pointed at the same hosts.
            prober: probe::Prober::new(PROBE_CONTACT_URL, &gateways)?,
            ipfs_gateways: gateways,
            per_client: RateLimiter::new("client", PER_CLIENT_QUOTAS),
            per_host: RateLimiter::new("target host", PER_HOST_QUOTAS),
            chains: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// Connect (or reuse) the clients for one chain.
    ///
    /// The registry address and chain id come from the `chains` table — the
    /// same row the indexer and sweeper read — so this endpoint can never be
    /// pointed at a contract the census does not use. The RPC URL comes from
    /// `RPC_URL_<CHAIN>`, exactly as every other binary here reads it; when it
    /// is unset the endpoint reports itself unavailable for that chain rather
    /// than failing in a way that looks like a bug.
    async fn clients(
        &self,
        db: &sqlx::PgPool,
        chain_name: &str,
    ) -> ApiResult<std::sync::Arc<ChainClients>> {
        let mut cache = self.chains.lock().await;
        if let Some(c) = cache.get(chain_name) {
            return Ok(c.clone());
        }

        let row: Option<(i64, String, Option<String>)> = sqlx::query_as(
            "SELECT chain_id, identity_registry, reputation_registry \
             FROM chains WHERE chain = $1 AND enabled",
        )
        .bind(chain_name)
        .fetch_optional(db)
        .await?;
        let (chain_id, identity_registry, reputation_registry) = row.ok_or(ApiError::NotFound)?;

        let rpc_var = format!("RPC_URL_{}", chain_name.to_uppercase());
        let rpc_url = std::env::var(&rpc_var).map_err(|_| {
            ApiError::Unavailable(format!(
                "spot checks are not configured for chain '{chain_name}' on this deployment \
                 ({rpc_var} is not set). Census results for this agent are still available at \
                 GET /api/agents/{chain_name}/{{id}}."
            ))
        })?;

        let registry = chain::Registry::connect(&rpc_url, &identity_registry)
            .await
            .map_err(|e| ApiError::Internal(format!("connecting to {chain_name}: {e:#}")))?;
        let reputation =
            match &reputation_registry {
                Some(addr) => Some(chain::Reputation::connect(&rpc_url, addr).await.map_err(
                    |e| {
                        ApiError::Internal(format!(
                            "connecting to {chain_name} reputation registry: {e:#}"
                        ))
                    },
                )?),
                None => None,
            };

        let clients = std::sync::Arc::new(ChainClients {
            registry,
            reputation,
            chain_id: chain_id as u64,
            identity_registry,
        });
        cache.insert(chain_name.to_string(), clients.clone());
        Ok(clients)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The handler
// ─────────────────────────────────────────────────────────────────────────────

/// `POST /api/agents/{chain}/{id}/spot-check`
///
/// The order of operations is the safety argument, so it is worth reading as
/// one:
///
/// 1. Parse the path. Nothing else happens for a malformed id.
/// 2. Charge the per-client limit — *before* any RPC call, so a refused caller
///    costs us nothing.
/// 3. Look the chain up and connect. 404 for an unknown chain, 503 for a chain
///    this deployment has no RPC for.
/// 4. Pin a block, then ask the registry whether this id exists. A `false`
///    here is a 404 and no request is ever sent anywhere.
/// 5. Read the snapshot and answer rung 1. If rung 1 does not pass, stop
///    before the fetch — `run_ladder` will mark rungs 2-5 and 7 `skipped`, so
///    the request would have bought an answer nobody is allowed to use.
/// 6. Charge the per-target-host limit, keyed on the URI the CHAIN gave us.
/// 7. Fetch, through the census's prober, and judge with the census's checks.
pub async fn post(
    State(state): State<AppState>,
    Path((chain_raw, id_raw)): Path<(String, String)>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> ApiResult<Json<SpotCheckResponse>> {
    let chain_name = normalise_chain(&chain_raw)?;
    let agent_id = parse_agent_id(&id_raw)?;
    let spot = &state.spot;

    // ── 2. Per-client limit, before we spend anything ────────────────────────
    let client = client_key(&headers, Some(peer), xff_depth());
    if let Err(wait) = spot.per_client.try_acquire(&client, Instant::now()) {
        return Err(spot.per_client.refuse(wait));
    }

    // ── 3. Chain config and clients ──────────────────────────────────────────
    let clients = spot.clients(&state.db, &chain_name).await?;

    // ── 4. Existence, at a pinned block ──────────────────────────────────────
    //
    // Pinned even though this is one agent: the two reads below (`ownerOf`,
    // `tokenURI`) must describe the same moment, and reading them at "latest"
    // twice can straddle a block. It is also what makes `block_number` in the
    // response a real answer to "as of when".
    let block = clients
        .registry
        .pinned_block()
        .await
        .map_err(|e| ApiError::Internal(format!("reading head block: {e:#}")))?;

    let exists = clients
        .registry
        .exists(agent_id, block)
        .await
        // Not a 404: an RPC failure is OUR problem and is not evidence that
        // the agent is absent. Conflating the two would let a flaky node
        // report a real agent as nonexistent.
        .map_err(|e| ApiError::Internal(format!("registry existence check: {e:#}")))?;
    if !exists {
        return Err(ApiError::NotFound);
    }

    let snapshot = clients
        .registry
        .snapshot(agent_id, block)
        .await
        .map_err(|e| ApiError::Internal(format!("reading agent {agent_id}: {e:#}")))?;

    let now = Utc::now();
    let rung1 = checks::registered(
        &checks::RegisteredInput {
            chain_id: clients.chain_id,
            registry: clients.identity_registry.clone(),
            token_id: snapshot.token_id.to_string(),
            owner: snapshot.owner.clone(),
            block_number: snapshot.block_number,
            // The census fills this from a `Registered` event scan over a whole
            // run. A single on-demand check does not scan logs — so `None`,
            // which the evidence records as null. Absent is absent; it is never
            // "the chain had nothing".
            tx_hash: None,
        },
        now,
    );
    // The gate on OUTBOUND TRAFFIC is deliberately its own decision, taken
    // from the owner address directly, rather than a reading of rung 1's
    // status. Rung 1 decides what to *report*; this decides whether to send a
    // stranger a request. Keeping them separate means a future change to
    // rung 1's rule cannot silently widen what this endpoint is willing to
    // probe for. The assertion pins that they agree today, and
    // `the_probe_gate_and_rung_1_agree_about_the_zero_address` pins it in CI.
    let probe_gate = existence_from_owner(&snapshot.owner);
    debug_assert_eq!(
        probe_gate == Existence::Registered,
        rung1.status == checks::CheckStatus::Pass,
        "the probe gate and rung 1 must agree about an unheld token"
    );

    // ── 5/6/7. Fetch and judge, but only if the ladder can use the answer ────
    let (fetch_summary, results) = if probe_gate == Existence::Unheld {
        // `ownerOf` did not revert but returned the zero address. Rung 1
        // fails, so `run_ladder` will mark rungs 2-5 (Document track) and 7
        // (Reputation track) `Skipped` whatever we put in them — which is
        // exactly why nothing is fetched and no reputation read is made. The
        // placeholder rungs below can never surface: `run_ladder` overwrites
        // every one of them, for the same reason the sweeper's
        // `assemble_ladder` may pass a `Null` document.
        let empty = probe::FetchOutcome {
            scheme: String::new(),
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
            inline_decode: None,
            gateway_attempts: Vec::new(),
        };
        (None, ladder(&clients, &snapshot, &empty, rung1, None, now))
    } else {
        // The per-host limit is charged against the host of the URI the CHAIN
        // returned — never against anything in the request — which is what
        // makes it un-evadable by rotating client addresses or agent ids.
        if let Some(host) = target_host(&snapshot.agent_uri, &spot.ipfs_gateways)
            && let Err(wait) = spot.per_host.try_acquire(&host, Instant::now())
        {
            return Err(spot.per_host.refuse(wait));
        }

        // The one and only outbound request, through the census's prober:
        // SSRF-guarded on the initial URL and on every redirect hop, robots
        // honored, per-host concurrency capped, timeouts and size cap applied.
        let outcome = spot.prober.fetch(&snapshot.agent_uri).await;

        // Rung 7 costs two RPC calls against a contract we own the address of.
        // No third party is involved, so it is not rate limited beyond the
        // per-client budget already charged.
        let feedback = match &clients.reputation {
            Some(rep) => match rep.feedback(agent_id, block).await {
                Ok(f) => Some(f),
                Err(e) => {
                    // OUR failure. Rather than record a false rung 7, leave it
                    // out entirely — absence means "not asked", which is the
                    // honest claim when the read did not complete.
                    tracing::warn!(
                        "spot check {chain_name}/{agent_id}: feedback read failed: {e:#}"
                    );
                    None
                }
            },
            None => None,
        };

        let results = ladder(&clients, &snapshot, &outcome, rung1, Some(feedback), now);
        (Some(summarise(&outcome)), results)
    };

    // Deliberately without the client key: this process keeps no record of who
    // asked for what (see the module doc's "Store or not store"). The host and
    // the agent are logged because they are what an operator complaining about
    // our traffic would name.
    tracing::info!(
        "spot check {chain_name}/{agent_id} at block {block}: {} checks asked",
        results.len()
    );

    Ok(Json(shape(ShapeInput {
        chain: chain_name,
        agent_id,
        block_number: block,
        chain_id: clients.chain_id,
        registry: clients.identity_registry.clone(),
        token_id: snapshot.token_id.to_string(),
        owner: snapshot.owner.clone(),
        // Verbatim, unlike the sweep, which escapes NUL bytes because Postgres
        // rejects them in `jsonb`. Nothing here is written to Postgres, JSON
        // strings may legally carry a NUL, and no rung's verdict depends on
        // the URI's text — so the honest thing is to report what the chain
        // actually holds.
        agent_uri: snapshot.agent_uri.clone(),
        fetch: fetch_summary,
        results,
        checked_at: now,
    })))
}

/// Build rungs 2 through 5 and 7 from what we observed, then hand every rung
/// to `checks::run_ladder`.
///
/// Skip-propagation is NOT decided here. `run_ladder` is the one place that
/// decides it, in this crate exactly as in the sweeper and in `/api/validate`
/// — three callers, one rule, no chance of three different answers.
///
/// `feedback` is doubly optional on purpose: the outer `None` means rung 7 was
/// not asked at all (rung 1 did not pass), the inner `None` means it was asked
/// and the read failed or the chain has no reputation registry. Only the
/// second produces a rung-7 row, and `checks::attested` decides its status
/// from `registry_available`.
fn ladder(
    clients: &ChainClients,
    snapshot: &chain::AgentSnapshot,
    outcome: &probe::FetchOutcome,
    rung1: checks::CheckResult,
    feedback: Option<Option<chain::FeedbackReads>>,
    now: DateTime<Utc>,
) -> Vec<checks::CheckResult> {
    // The one definition of the scheme bucket, shared with the sweeper — see
    // `probe::FetchOutcome::scheme_bucket`.
    let scheme = outcome.scheme_bucket();
    let inline_bytes = if scheme == "data" {
        outcome.body.as_ref().map(Vec::len)
    } else {
        None
    };

    let rung2 = checks::resolvable(
        &checks::ResolvableInput {
            uri: snapshot.agent_uri.clone(),
            scheme,
            request_url: outcome.request_url.clone(),
            final_url: outcome.final_url.clone(),
            http_status: outcome.http_status,
            elapsed_ms: outcome.elapsed_ms,
            error: outcome.error.clone(),
            inline_bytes,
            via_gateway: outcome.via_gateway.clone(),
            inline_decode_variant: outcome
                .inline_decode
                .as_ref()
                .map(|d| d.variant.to_string()),
            inline_decode_algorithm: outcome
                .inline_decode
                .as_ref()
                .and_then(|d| d.algorithm.clone()),
            gateway_attempts: if outcome.gateway_attempts.is_empty() {
                None
            } else {
                serde_json::to_value(&outcome.gateway_attempts).ok()
            },
        },
        now,
    );

    let (rung3, document) = checks::parseable(
        &checks::ParseableInput {
            body: outcome.body.clone(),
            content_type: outcome.content_type.clone(),
            body_sha256: outcome.body_sha256.clone(),
            truncated: outcome.truncated,
        },
        now,
    );

    // Constructed unconditionally, exactly as the sweeper does since P0 FIX 6:
    // a document that never parsed must produce `skipped` rungs 4 and 5, and
    // only `run_ladder` is allowed to decide that. The `Null` placeholder can
    // never surface, because `document.is_none()` implies rung 3 did not pass.
    let document = document.unwrap_or(serde_json::Value::Null);
    let rung4 = checks::conformant(
        &checks::ConformantInput {
            document: document.clone(),
        },
        checks::SPEC_COMMIT,
        now,
    );
    let rung5 = checks::bound(
        &checks::BoundInput {
            document,
            actual_agent_id: snapshot.agent_id,
            actual_chain_id: clients.chain_id,
            actual_registry: clients.identity_registry.clone(),
        },
        now,
    );

    let mut rungs = vec![rung1, rung2, rung3, rung4, rung5];

    // Rung 6 is never constructed. Absence is the claim — see NOT_CHECKED_RUNG6.

    if let Some(feedback) = feedback {
        match feedback {
            Some(f) => rungs.push(checks::attested(
                &checks::AttestedInput {
                    clients: f.clients,
                    feedback_count: f.feedback_count,
                    registry_available: true,
                },
                now,
            )),
            None if clients.reputation.is_none() => {
                // The chain has no Reputation Registry configured: our
                // limitation, not the agent's. `checks::attested` reads that
                // off `registry_available` and answers `error`, never `fail`.
                rungs.push(checks::attested(
                    &checks::AttestedInput {
                        clients: Vec::new(),
                        feedback_count: 0,
                        registry_available: false,
                    },
                    now,
                ));
            }
            // The read was attempted and failed. No row: "we did not get an
            // answer" is not one of the six statuses, and absence already
            // means it.
            None => {}
        }
    }

    checks::run_ladder(rungs)
}

/// Reduce a `FetchOutcome` to what a reader can check, dropping the body.
fn summarise(outcome: &probe::FetchOutcome) -> SpotFetch {
    SpotFetch {
        scheme: outcome.scheme_bucket(),
        request_url: outcome.request_url.clone(),
        final_url: outcome.final_url.clone(),
        http_status: outcome.http_status,
        content_type: outcome.content_type.clone(),
        body_bytes: outcome.body.as_ref().map(Vec::len),
        body_sha256: outcome.body_sha256.clone(),
        truncated: outcome.truncated,
        error: outcome.error.clone(),
        elapsed_ms: outcome.elapsed_ms,
        via_gateway: outcome.via_gateway.clone(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests — everything pure. No database, no RPC, no network.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    fn res(rung: u8, name: &'static str, status: checks::CheckStatus) -> checks::CheckResult {
        checks::CheckResult {
            rung,
            name,
            status,
            evidence: serde_json::json!({}),
            checked_at: t(),
        }
    }

    // ── The rate limiter ────────────────────────────────────────────────────

    const ONE: &[Quota] = &[Quota {
        limit: 3,
        window: Duration::from_secs(60),
    }];

    const TWO: &[Quota] = &[
        Quota {
            limit: 1,
            window: Duration::from_secs(60),
        },
        Quota {
            limit: 3,
            window: Duration::from_secs(3600),
        },
    ];

    #[test]
    fn the_limit_allows_exactly_its_budget_and_then_refuses() {
        let l = RateLimiter::new("test", ONE);
        let t0 = Instant::now();
        for i in 0..3 {
            assert!(
                l.try_acquire("k", t0).is_ok(),
                "call {i} must be inside the budget"
            );
        }
        assert!(l.try_acquire("k", t0).is_err(), "the 4th must be refused");
    }

    #[test]
    fn a_refusal_says_when_the_oldest_hit_leaves_the_window() {
        let l = RateLimiter::new("test", ONE);
        let t0 = Instant::now();
        for _ in 0..3 {
            l.try_acquire("k", t0).unwrap();
        }
        // 10s later, the oldest hit still has 50s to live.
        let wait = l
            .try_acquire("k", t0 + Duration::from_secs(10))
            .expect_err("still over budget");
        assert_eq!(wait, Duration::from_secs(50));
    }

    #[test]
    fn budget_returns_once_the_window_has_passed() {
        let l = RateLimiter::new("test", ONE);
        let t0 = Instant::now();
        for _ in 0..3 {
            l.try_acquire("k", t0).unwrap();
        }
        assert!(l.try_acquire("k", t0 + Duration::from_secs(59)).is_err());
        assert!(
            l.try_acquire("k", t0 + Duration::from_secs(60)).is_ok(),
            "a hit exactly one window old no longer counts"
        );
    }

    /// The rule that makes a limiter fair under load: being refused must not
    /// extend your own penalty. If rejected attempts were recorded, a client
    /// retrying in a loop could never get back in.
    #[test]
    fn a_refused_attempt_is_not_itself_counted() {
        let l = RateLimiter::new("test", ONE);
        let t0 = Instant::now();
        for _ in 0..3 {
            l.try_acquire("k", t0).unwrap();
        }
        for s in 1..50 {
            let _ = l.try_acquire("k", t0 + Duration::from_secs(s));
        }
        // Those 49 refusals recorded nothing, so the original window still
        // governs and expires on schedule.
        assert!(l.try_acquire("k", t0 + Duration::from_secs(60)).is_ok());
    }

    #[test]
    fn two_quotas_bind_independently_and_the_longer_wait_wins() {
        let l = RateLimiter::new("test", TWO);
        let t0 = Instant::now();
        // The 1-per-minute rule binds first.
        l.try_acquire("k", t0).unwrap();
        assert_eq!(
            l.try_acquire("k", t0 + Duration::from_secs(30)),
            Err(Duration::from_secs(30))
        );
        // Spaced a minute apart, three get through — then the hourly cap binds.
        l.try_acquire("k", t0 + Duration::from_secs(60)).unwrap();
        l.try_acquire("k", t0 + Duration::from_secs(120)).unwrap();
        let wait = l
            .try_acquire("k", t0 + Duration::from_secs(180))
            .expect_err("the hourly cap must bind even though a minute has passed");
        // The oldest of the three hits was at t0, so the hour frees up at
        // t0+3600, i.e. 3420s after t0+180.
        assert_eq!(wait, Duration::from_secs(3420));
    }

    #[test]
    fn distinct_keys_do_not_share_a_budget() {
        let l = RateLimiter::new("test", TWO);
        let t0 = Instant::now();
        l.try_acquire("host-a", t0).unwrap();
        assert!(l.try_acquire("host-a", t0).is_err());
        assert!(
            l.try_acquire("host-b", t0).is_ok(),
            "one host's budget must not spend another's"
        );
    }

    /// The production numbers, pinned. If someone loosens these, the test that
    /// fails should be the one that states what they were for.
    #[test]
    fn the_published_limits_are_the_ones_in_force() {
        assert_eq!(PER_CLIENT_QUOTAS.len(), 1);
        assert_eq!(PER_CLIENT_QUOTAS[0].limit, 5);
        assert_eq!(PER_CLIENT_QUOTAS[0].window, Duration::from_secs(600));

        assert_eq!(PER_HOST_QUOTAS.len(), 2);
        assert_eq!(PER_HOST_QUOTAS[0].limit, 1);
        assert_eq!(PER_HOST_QUOTAS[0].window, Duration::from_secs(60));
        assert_eq!(PER_HOST_QUOTAS[1].limit, 20);
        assert_eq!(PER_HOST_QUOTAS[1].window, Duration::from_secs(3600));

        // The per-host limit must be the tighter of the two, or the courtesy
        // limit would be doing the protecting.
        let host_per_hour = PER_HOST_QUOTAS[1].limit as f64;
        let client_per_hour = PER_CLIENT_QUOTAS[0].limit as f64 * 6.0;
        assert!(
            host_per_hour <= client_per_hour * 2.0,
            "the per-host budget must stay in the same order as one client's"
        );
    }

    #[test]
    fn a_sub_second_wait_still_asks_for_at_least_one_second() {
        let l = RateLimiter::new("client", PER_CLIENT_QUOTAS);
        match l.refuse(Duration::from_millis(1)) {
            ApiError::TooManyRequests {
                retry_after_secs, ..
            } => assert_eq!(retry_after_secs, 1),
            other => panic!("expected a 429, got {other:?}"),
        }
    }

    #[test]
    fn the_refusal_names_the_limit_and_points_at_the_unlimited_census_route() {
        let l = RateLimiter::new("target host", PER_HOST_QUOTAS);
        match l.refuse(Duration::from_secs(42)) {
            ApiError::TooManyRequests {
                retry_after_secs,
                message,
            } => {
                assert_eq!(retry_after_secs, 42);
                assert!(message.contains("target host"), "{message}");
                assert!(message.contains("/api/agents/"), "{message}");
            }
            other => panic!("expected a 429, got {other:?}"),
        }
    }

    // ── The client key ──────────────────────────────────────────────────────

    fn headers_with(xff: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", xff.parse().unwrap());
        h
    }

    fn peer() -> Option<SocketAddr> {
        Some("203.0.113.9:51000".parse().unwrap())
    }

    #[test]
    fn the_forwarded_chain_is_read_from_the_right() {
        // Only the rightmost entry was written by a hop we are behind; the
        // caller can prepend anything they like to the left of it.
        let h = headers_with("1.2.3.4, 198.51.100.7");
        assert_eq!(client_key(&h, peer(), 1), "198.51.100.7");
    }

    #[test]
    fn a_spoofed_prefix_cannot_choose_the_bucket() {
        let honest = client_key(&headers_with("198.51.100.7"), peer(), 1);
        let spoofed = client_key(&headers_with("9.9.9.9, 198.51.100.7"), peer(), 1);
        assert_eq!(honest, spoofed);
    }

    #[test]
    fn a_deeper_proxy_chain_is_configurable() {
        // Behind a load balancer: `…, <client>, <lb>`.
        let h = headers_with("1.2.3.4, 198.51.100.7, 10.0.0.1");
        assert_eq!(client_key(&h, peer(), 2), "198.51.100.7");
    }

    #[test]
    fn a_depth_deeper_than_the_chain_degrades_to_over_limiting() {
        // Saturating to the leftmost entry keeps everyone who shares that
        // prefix in one bucket. That is the safe direction to be wrong in.
        let h = headers_with("198.51.100.7");
        assert_eq!(client_key(&h, peer(), 4), "198.51.100.7");
    }

    #[test]
    fn the_peer_address_is_used_when_nothing_is_proxying() {
        assert_eq!(client_key(&HeaderMap::new(), peer(), 1), "203.0.113.9");
    }

    #[test]
    fn an_empty_forwarded_header_falls_back_rather_than_keying_on_nothing() {
        assert_eq!(client_key(&headers_with(" , "), peer(), 1), "203.0.113.9");
        assert_eq!(client_key(&HeaderMap::new(), None, 1), "unknown");
    }

    // ── The target host ─────────────────────────────────────────────────────

    fn gateways() -> Vec<String> {
        vec![
            "https://ipfs.io/ipfs/".to_string(),
            "https://cloudflare-ipfs.com/ipfs/".to_string(),
        ]
    }

    #[test]
    fn an_http_uri_is_limited_by_its_own_host() {
        assert_eq!(
            target_host("https://Example.COM/agent.json", &gateways()).as_deref(),
            Some("example.com"),
            "the key must be case-folded or two spellings would be two budgets"
        );
    }

    #[test]
    fn an_ipfs_uri_is_limited_by_the_gateway_it_will_hit() {
        assert_eq!(
            target_host("ipfs://bafyfakecid/card.json", &gateways()).as_deref(),
            Some("ipfs.io")
        );
    }

    #[test]
    fn a_uri_that_sends_no_request_is_not_limited() {
        // The ~65% garbage bucket plus inline documents: no third party is
        // touched, so limiting them would only stop people learning that.
        for quiet in [
            "",
            "   ",
            "data:application/json;base64,eyJhIjoxfQ==",
            "undefined/agents/442/agent-card/v1",
            "ftp://example.com/x",
        ] {
            assert_eq!(
                target_host(quiet, &gateways()),
                None,
                "{quiet:?} makes no request and needs no host budget"
            );
        }
    }

    /// A private-range host still gets a bucket. The netguard is what refuses
    /// it; the limiter is not a second, weaker SSRF check and must not be
    /// mistaken for one.
    #[test]
    fn a_private_host_is_still_keyed_and_left_to_the_netguard() {
        assert_eq!(
            target_host("http://169.254.169.254/latest/meta-data/", &gateways()).as_deref(),
            Some("169.254.169.254")
        );
    }

    // ── The "agent must exist" guard ────────────────────────────────────────

    #[test]
    fn an_agent_id_must_be_a_whole_number_in_range() {
        assert_eq!(parse_agent_id("0").unwrap(), 0);
        assert_eq!(parse_agent_id(" 442 ").unwrap(), 442);
        for bad in ["-1", "1.5", "0x1a", "", "abc", "99999999999999999999999"] {
            assert!(
                parse_agent_id(bad).is_err(),
                "{bad:?} must not reach the registry"
            );
        }
    }

    #[test]
    fn a_bad_agent_id_is_a_400_that_names_the_value() {
        match parse_agent_id("-1") {
            Err(ApiError::BadRequest(m)) => assert!(m.contains("-1"), "{m}"),
            other => panic!("expected a 400, got {other:?}"),
        }
    }

    #[test]
    fn a_chain_name_is_normalised_and_shape_checked() {
        assert_eq!(normalise_chain(" BASE ").unwrap(), "base");
        assert_eq!(normalise_chain("bnb-chain").unwrap(), "bnb-chain");
        for bad in [
            "",
            "   ",
            "base; DROP TABLE runs",
            "../etc",
            "base chain",
            "b".repeat(33).as_str(),
        ] {
            assert!(normalise_chain(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn a_token_nobody_holds_is_not_an_agent_we_probe_for() {
        assert_eq!(
            existence_from_owner("0x0000000000000000000000000000000000000000"),
            Existence::Unheld
        );
        // Case and whitespace must not create a hole in the guard.
        assert_eq!(
            existence_from_owner("  0X0000000000000000000000000000000000000000 "),
            Existence::Unheld
        );
        assert_eq!(
            existence_from_owner("0xabc0000000000000000000000000000000000001"),
            Existence::Registered
        );
    }

    /// The guard and rung 1 must agree, because the handler uses the first to
    /// decide whether to send a request and the second to report a verdict.
    /// If they ever disagreed, we would either probe on behalf of a token
    /// nobody holds or refuse to probe for an agent we then call registered.
    #[test]
    fn the_probe_gate_and_rung_1_agree_about_the_zero_address() {
        for owner in [
            "0x0000000000000000000000000000000000000000",
            "0xabc0000000000000000000000000000000000001",
        ] {
            let r = checks::registered(
                &checks::RegisteredInput {
                    chain_id: 8453,
                    registry: "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432".into(),
                    token_id: "42".into(),
                    owner: owner.into(),
                    block_number: 1,
                    tx_hash: None,
                },
                t(),
            );
            let gate = existence_from_owner(owner) == Existence::Registered;
            assert_eq!(gate, r.status == checks::CheckStatus::Pass, "for {owner}");
        }
    }

    // ── Response shaping ────────────────────────────────────────────────────

    fn shape_input(results: Vec<checks::CheckResult>) -> ShapeInput {
        ShapeInput {
            chain: "base".into(),
            agent_id: 442,
            block_number: 41_817_815,
            chain_id: 8453,
            registry: "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432".into(),
            token_id: "442".into(),
            owner: "0xabc0000000000000000000000000000000000001".into(),
            agent_uri: "https://example.com/agent.json".into(),
            fetch: None,
            results,
            checked_at: t(),
        }
    }

    fn full_ladder() -> Vec<checks::CheckResult> {
        checks::run_ladder(vec![
            res(1, "registered", checks::CheckStatus::Pass),
            res(2, "resolvable", checks::CheckStatus::Pass),
            res(3, "parseable", checks::CheckStatus::Pass),
            res(4, "conformant", checks::CheckStatus::Pass),
            res(5, "bound", checks::CheckStatus::Pass),
            res(7, "attested", checks::CheckStatus::Pass),
        ])
    }

    #[test]
    fn every_response_carries_the_discriminator_and_the_notice() {
        let r = shape(shape_input(full_ladder()));
        assert_eq!(r.source, "spot_check");
        assert!(r.notice.contains("NOT a census measurement"));
        // Its own provenance, so it can be told from a census row on sight.
        assert_eq!(r.block_number, 41_817_815);
        assert_eq!(r.checked_at, t());
        assert_eq!(r.checker_version, checks::CHECKER_VERSION);
        assert_eq!(r.spec_commit, checks::SPEC_COMMIT);
        assert!(
            !r.checker_commit.is_empty(),
            "the build must stamp a commit, even if it is 'unknown'"
        );
    }

    /// The invariant with teeth: a spot check belongs to no run, so no field
    /// anywhere in the body may name one. A screenshot with a `run_id` in it
    /// is a census row as far as any reader is concerned.
    #[test]
    fn no_response_field_anywhere_names_a_run() {
        let json = serde_json::to_string(&shape(shape_input(full_ladder()))).unwrap();
        assert!(!json.contains("run_id"), "{json}");
        assert!(!json.contains("\"run\""), "{json}");
    }

    /// The top-level keys are pinned against `routes::agents::AgentDetail`'s.
    /// The two shapes must not converge by accident as either one grows.
    #[test]
    fn the_shape_is_not_the_census_agent_detail_shape() {
        let v = serde_json::to_value(shape(shape_input(full_ladder()))).unwrap();
        let obj = v.as_object().unwrap();
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "agent_id",
                "block_number",
                "chain",
                "checked_at",
                "checker_commit",
                "checker_version",
                "checks",
                "fetch",
                "identity",
                "not_checked",
                "notice",
                "schema_version",
                "source",
                "spec_commit",
            ]
        );
        // The census detail's own names, absent on purpose.
        assert!(obj.get("rungs").is_none(), "census uses `rungs`");
        assert!(obj.get("archive").is_none(), "census uses `archive`");
        assert!(obj.get("snapshot").is_none(), "census uses `snapshot`");
    }

    #[test]
    fn rung_6_is_absent_from_the_checks_and_explained_in_not_checked() {
        let r = shape(shape_input(full_ladder()));
        assert!(
            !r.checks.iter().any(|c| c.rung == 6),
            "a rung nobody asked must have no entry — never a guessed status"
        );
        let six = r
            .not_checked
            .iter()
            .find(|n| n.rung == 6)
            .expect("rung 6 must be accounted for");
        assert_eq!(six.name, "live");
        assert!(six.reason.contains("per-host budget"), "{}", six.reason);
    }

    #[test]
    fn a_rung_that_was_asked_is_never_also_listed_as_not_checked() {
        let r = shape(shape_input(full_ladder()));
        for c in &r.checks {
            assert!(
                !r.not_checked.iter().any(|n| n.rung == c.rung),
                "rung {} is in both lists",
                c.rung
            );
        }
        // Every rung is accounted for exactly once, one way or the other.
        let mut seen: Vec<u8> = r
            .checks
            .iter()
            .map(|c| c.rung)
            .chain(r.not_checked.iter().map(|n| n.rung))
            .collect();
        seen.sort_unstable();
        assert_eq!(seen, [1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn a_missing_rung_7_says_why_rather_than_disappearing() {
        // The rung-1-failed path: nothing fetched, no reputation read.
        let results = checks::run_ladder(vec![
            res(1, "registered", checks::CheckStatus::Fail),
            res(2, "resolvable", checks::CheckStatus::Pass),
            res(3, "parseable", checks::CheckStatus::Pass),
            res(4, "conformant", checks::CheckStatus::Pass),
            res(5, "bound", checks::CheckStatus::Pass),
        ]);
        let r = shape(shape_input(results));
        let seven = r.not_checked.iter().find(|n| n.rung == 7).unwrap();
        assert_eq!(seven.name, "attested");
        assert!(seven.reason.contains("rung 1 did not pass"));
        // And the rungs that WERE asked carry the ladder's verdict, not ours.
        let two = r.checks.iter().find(|c| c.rung == 2).unwrap();
        assert_eq!(two.status, "skipped");
    }

    #[test]
    fn checks_come_back_in_rung_order_whatever_order_they_were_built_in() {
        let r = shape(shape_input(vec![
            res(5, "bound", checks::CheckStatus::Pass),
            res(1, "registered", checks::CheckStatus::Pass),
            res(3, "parseable", checks::CheckStatus::Pass),
        ]));
        assert_eq!(
            r.checks.iter().map(|c| c.rung).collect::<Vec<_>>(),
            [1, 3, 5]
        );
    }

    /// The statuses on the wire are `crates/checks`' own words, not a second
    /// vocabulary. `unclaimed` and `unprobeable` in particular must survive.
    #[test]
    fn statuses_are_the_checkers_words_verbatim() {
        let r = shape(shape_input(vec![
            res(1, "registered", checks::CheckStatus::Pass),
            res(5, "bound", checks::CheckStatus::Unclaimed),
        ]));
        assert_eq!(r.checks[1].status, "unclaimed");
    }

    // ── The SSRF guard is on THIS path ──────────────────────────────────────

    /// The endpoint's inputs are attacker-controlled on-chain strings: anyone
    /// can register an agent whose `tokenURI()` is `http://169.254.169.254/…`
    /// and then ask us to fetch it for them. The guard that refuses is
    /// `probe`'s netguard, and this test exercises it through the exact call
    /// the handler makes — `Prober::fetch`, built by `SpotCheckService::new`
    /// with the same arguments — rather than through the netguard directly,
    /// because the thing worth pinning is that this path reaches the guard at
    /// all.
    ///
    /// No network is involved: every address below is a literal IP, which
    /// `netguard::resolve` rejects before DNS, before robots.txt and before
    /// any connection is attempted.
    #[tokio::test]
    async fn the_probe_this_endpoint_uses_refuses_every_non_public_address() {
        let service = SpotCheckService::new().expect("the service must build");
        for hostile in [
            // The cloud metadata endpoint — the classic SSRF target.
            "http://169.254.169.254/latest/meta-data/",
            "http://127.0.0.1:8080/admin",
            "http://localhost:5432/",
            "http://10.0.0.5/internal",
            "http://172.16.0.1/internal",
            "http://192.168.1.1/router",
            // RFC 6598 CGNAT, used for cloud-internal networks.
            "http://100.64.0.1/",
            "http://0.0.0.0/",
            "https://[::1]/admin",
            // IPv6 unique-local (fc00::/7) and link-local (fe80::/10).
            "https://[fc00::1]/",
            "https://[fe80::1]/",
            // A v4-mapped v6 address smuggling a private v4 through.
            "https://[::ffff:10.0.0.1]/",
        ] {
            let outcome = service.prober.fetch(hostile).await;
            let error = outcome.error.unwrap_or_default();
            assert!(
                error.starts_with("ssrf_blocked"),
                "{hostile} must be refused by the netguard, got {error:?}"
            );
            assert!(
                outcome.http_status.is_none() && outcome.body.is_none(),
                "{hostile} must not have been connected to at all"
            );
        }
    }

    /// `localhost` resolves through DNS rather than as a literal, so it
    /// exercises the guard's resolve-then-check branch: the host is looked up
    /// and refused because an address it resolves to is not public. Separate
    /// from the literal cases above because it is the branch a hostile DNS
    /// record would use.
    #[tokio::test]
    async fn a_hostname_that_resolves_into_a_private_range_is_refused_too() {
        let service = SpotCheckService::new().expect("the service must build");
        let outcome = service.prober.fetch("http://localhost/agent.json").await;
        let error = outcome.error.unwrap_or_default();
        assert!(error.starts_with("ssrf_blocked"), "got {error:?}");
    }

    /// Redirects are followed — disabling them made an ordinary 301 look like
    /// a failure — so every hop has to be re-validated, or a public host could
    /// bounce us into the metadata service. The re-validation lives in
    /// `probe::Prober::fetch_http`'s hop loop and is what `MAX_REDIRECTS`
    /// bounds. This pins that the spot check inherits both, rather than a
    /// prober configured to follow redirects unchecked.
    #[test]
    fn the_redirect_cap_the_spot_check_inherits_is_the_censuss() {
        assert_eq!(probe::MAX_REDIRECTS, 3);
        assert_eq!(probe::PER_HOST_CAP, 2);
        assert_eq!(probe::MAX_BODY_BYTES, 1024 * 1024);
    }

    #[test]
    fn evidence_is_carried_through_untouched() {
        let mut r5 = res(5, "bound", checks::CheckStatus::Fail);
        r5.evidence = serde_json::json!({"declared_registry": "0xdead", "matched": false});
        let out = shape(shape_input(vec![
            res(1, "registered", checks::CheckStatus::Pass),
            r5,
        ]));
        assert_eq!(out.checks[1].evidence["declared_registry"], "0xdead");
        assert_eq!(out.checks[1].evidence["matched"], false);
    }
}
