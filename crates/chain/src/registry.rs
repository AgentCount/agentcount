//! Reading the Identity Registry: which agents exist, and what the chain says
//! about each one RIGHT NOW.
//!
//! Rust concept spotlight: **`sol!` generates call types, not just events.**
//! Given a function signature it produces a typed builder, so `ownerOf(id)`
//! is a compile-checked call rather than hand-packed calldata.
//!
//! ## Why agent ids are enumerated by binary search, not by scanning logs
//!
//! An earlier version of this module discovered agent ids by scanning
//! `Registered` logs over `[deploy_block, head]`. That doesn't work on a
//! free-tier RPC: Alchemy's free tier caps `eth_getLogs` at a **10-block**
//! range (verbatim: "Under the Free tier plan, you can make eth_getLogs
//! requests with up to a 10 block range"), and the deploy-to-head span on
//! Base is on the order of 7.5M blocks — roughly 753,000 calls, which is not
//! a chunk-size problem to adaptively split around; it's a hard ceiling set
//! by the plan, not the request.
//!
//! Agent ids in this registry are instead contiguous integers starting at 0,
//! so [`Registry::highest_agent_id`] finds the top of that range by binary
//! search on `ownerOf` existence (~17 calls for a ~60,000-agent registry,
//! growing only as O(log population)), and [`Registry::enumerate_agent_ids`]
//! walks `0..=max`. This also changes what's being counted: it's a census of
//! agents that *currently exist*, not a replay of everything ever minted —
//! the right thing for "what does the registry say right now", and
//! consistent with this crate's read-current-state approach elsewhere.
//!
//! The `Registered` event below is kept only as documentation of what the
//! registry emits on mint; nothing in this module decodes it any more.

use alloy::eips::BlockId;
use alloy::primitives::{Address, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::sol;
use anyhow::{Context, Result};

sol! {
    #[sol(rpc)]
    interface IIdentityRegistry {
        function ownerOf(uint256 tokenId) external view returns (address);
        function tokenURI(uint256 tokenId) external view returns (string);
    }

    // Documents what the registry emits on mint. Not decoded anywhere in this
    // module — see the module doc comment for why log-based discovery was
    // abandoned in favour of binary search over `ownerOf`.
    event Registered(uint256 indexed agentId, string agentURI, address indexed owner);
}

/// Does this RPC error mean "the provider is throttling us right now", as
/// opposed to a genuine, permanent problem with the request itself? Alchemy's
/// free tier returns HTTP 429 with "exceeded its compute units per second
/// capacity" under sustained load — exactly what a ~60,000-agent sweep
/// generates. Retrying THIS is correct because the request was fine and the
/// provider will accept it again shortly; retrying a malformed call or a
/// contract revert forever would not (those never match here).
fn is_throttled(e: &impl std::fmt::Display) -> bool {
    let s = e.to_string().to_lowercase();
    s.contains("429")
        || s.contains("compute units per second")
        || s.contains("too many requests")
        || s.contains("rate limit")
}

/// Does this `ownerOf` failure mean "this token id does not exist" (a
/// contract revert), as opposed to "we don't know" (a transport error, a
/// throttled request, a malformed response)? Verified empirically against
/// Base on 2026-07-27: calling `ownerOf` on a far-out, definitely-unminted id
/// (999,999,999) returns a JSON-RPC error response with `code: 3` and message
/// `execution reverted` — exactly the shape ERC-721 registries use for a
/// non-existent token.
///
/// This distinction matters more than it looks: [`Registry::exists`] maps
/// `is_revert` to `Ok(false)` and everything else to `Err`. If a 429 or a
/// dropped connection were misread as "does not exist", binary search in
/// [`Registry::highest_agent_id`] would silently report a far smaller
/// population than reality — a wrong census that looks like a correct one.
/// The `!is_throttled` guard is defence in depth, mirroring how
/// `is_range_too_large` used to exclude throttling in the log-scan approach
/// this replaced: throttling messages happen not to contain "execution
/// reverted" today, but the exclusion costs nothing and keeps the two checks
/// from ever competing to explain the same message.
fn is_revert(e: &impl std::fmt::Display) -> bool {
    let s = e.to_string().to_lowercase();
    s.contains("execution reverted") && !is_throttled(e)
}

/// How many times to retry a throttled call before giving up and letting the
/// caller treat it as a real failure. Bounded so a provider that is down
/// outright (not just busy) still fails within a reasonable time instead of
/// spinning forever.
const MAX_THROTTLE_RETRIES: u32 = 8;
/// Base of the exponential backoff, doubled each retry: 300ms, 600ms, 1.2s,
/// 2.4s, 4.8s, 9.6s, 19.2s, 38.4s — generous, because "busy" on a free-tier
/// RPC can last several seconds, and a completed sweep beats a fast one.
const THROTTLE_BACKOFF_BASE_MS: u64 = 300;

/// Run `f` and, if it fails because the provider is throttling us
/// ([`is_throttled`]), retry with exponential backoff up to
/// `MAX_THROTTLE_RETRIES` times. Any other error — a genuine one — returns
/// immediately on the first attempt: throttling is the only condition where
/// "ask again" is the right response to "no".
async fn retry_throttled<T, E, F, Fut>(f: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    retry_throttled_with(MAX_THROTTLE_RETRIES, THROTTLE_BACKOFF_BASE_MS, f).await
}

/// The tunable core of [`retry_throttled`], parameterised on retry budget and
/// backoff base so tests can exercise "gives up eventually" in milliseconds
/// instead of the real ~76s the production constants would take.
async fn retry_throttled_with<T, E, F, Fut>(
    max_retries: u32,
    backoff_base_ms: u64,
    mut f: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempt = 0u32;
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if is_throttled(&e) && attempt < max_retries => {
                let backoff_ms = backoff_base_ms * (1u64 << attempt);
                tracing::warn!(
                    "throttled (attempt {}/{max_retries}), backing off {backoff_ms}ms: {e:#}",
                    attempt + 1
                );
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Binary-search the boundary between "exists" and "does not exist" over a
/// predicate `p`, given a starting point `lo` that the caller already knows
/// exists (`p(lo)` is assumed true — not re-checked here). Returns `(lo, hi)`
/// with `hi == lo + 1`, `p(lo)` true and `p(hi)` false.
///
/// Doubles `hi` from `lo + 1` until it finds a point where `p` is false (a
/// bracket), then bisects — so the total number of `p` calls is
/// O(log(final value)) regardless of how large the registry has grown,
/// rather than a fixed guess that would need retuning as the population
/// grows.
///
/// Kept free of any network type (`p` is a generic async predicate) so the
/// search logic itself is unit-testable without an RPC connection; see the
/// tests below.
async fn find_boundary<P, Fut>(mut lo: u64, mut p: P) -> Result<(u64, u64)>
where
    P: FnMut(u64) -> Fut,
    Fut: std::future::Future<Output = Result<bool>>,
{
    let mut hi = lo
        .checked_add(1)
        .context("id overflowed u64 starting the search bracket")?;
    loop {
        if !p(hi).await? {
            break;
        }
        lo = hi;
        hi = hi
            .checked_mul(2)
            .context("agent id search overflowed u64 while doubling the bracket")?;
    }
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if p(mid).await? {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok((lo, hi))
}

/// What the chain says about one agent at one block.
#[derive(Debug, Clone)]
pub struct AgentSnapshot {
    pub agent_id: u64,
    pub token_id: U256,
    /// Lowercase hex. Normalised here so nothing downstream has to remember to.
    pub owner: String,
    /// From `tokenURI()`. An empty string is a legitimate on-chain value and
    /// must be preserved as such — it is a finding, not a missing read.
    pub agent_uri: String,
    pub block_number: u64,
}

pub struct Registry {
    // `DynProvider`, not the concrete builder-fill-stack type: `ProviderBuilder::new()`
    // returns a `FillProvider<JoinFill<...>, RootProvider>` whose exact type is a
    // mouthful (and not meant to be spelled out in a struct field). `.erased()`
    // boxes it into one nameable type — same trick `crates/indexer/src/chains.rs`
    // already uses for exactly this reason.
    provider: DynProvider,
    address: Address,
}

impl Registry {
    pub async fn connect(rpc_url: &str, address: &str) -> Result<Self> {
        let provider = ProviderBuilder::new()
            .connect(rpc_url)
            .await
            .with_context(|| format!("connecting to {rpc_url}"))?
            .erased();
        let address: Address = address.parse().context("parsing registry address")?;
        Ok(Self { provider, address })
    }

    /// The block every read in this run is pinned to. Pinning matters: an
    /// unpinned sweep reads agent 1 at block N and agent 9000 at block N+40,
    /// and the census is then of a population that never simultaneously existed.
    pub async fn pinned_block(&self) -> Result<u64> {
        Ok(self.provider.get_block_number().await?)
    }

    /// Does agent `agent_id` currently exist, as of `block`? Reads
    /// `ownerOf(agent_id)` pinned to `block`, routed through
    /// [`retry_throttled`] so a 429 is retried rather than misread as an
    /// answer. A revert ([`is_revert`]) means "no such token" and becomes
    /// `Ok(false)`; any other failure is NOT evidence of non-existence and
    /// propagates as `Err` — see [`is_revert`]'s doc comment for why
    /// conflating the two would be dangerous here.
    async fn exists(&self, agent_id: u64, block: u64) -> Result<bool> {
        let c = IIdentityRegistry::new(self.address, &self.provider);
        let token_id = U256::from(agent_id);
        match retry_throttled(|| async {
            c.ownerOf(token_id).block(BlockId::from(block)).call().await
        })
        .await
        {
            Ok(_owner) => Ok(true),
            Err(e) if is_revert(&e) => Ok(false),
            Err(e) => Err(e).with_context(|| format!("ownerOf({agent_id}) existence check")),
        }
    }

    /// The highest agent id that currently exists as of `block`, `None` if
    /// id 0 does not exist (an empty registry).
    ///
    /// **Contiguity assumption**: this registry mints agent ids as a
    /// contiguous sequence starting at 0. Verified empirically against Base
    /// on 2026-07-27 by spot-checking ids 0, 1, 7, 999, 2116, 8888, 15000,
    /// 27777, 33333, 41000, 52000, 59997 — all existed — while 59998 and
    /// 60001 both reverted, either side of a binary-search boundary at
    /// 59997. Neither `totalSupply()` (reverts on this contract) nor
    /// ERC721Enumerable (`supportsInterface(0x780e9d63)` returns false) is
    /// available to cross-check this independently.
    ///
    /// If ids ever become sparse — some id below the max deliberately never
    /// minted, or burned and not reused — [`enumerate_agent_ids`] would
    /// over-report: it walks `0..=max` and would list ids that don't exist.
    /// This function's boundary assertion below only guards the top of the
    /// range; it cannot detect a hole in the middle.
    ///
    /// [`enumerate_agent_ids`]: Registry::enumerate_agent_ids
    pub async fn highest_agent_id(&self, block: u64) -> Result<Option<u64>> {
        if !self.exists(0, block).await? {
            return Ok(None);
        }

        let (lo, hi) = find_boundary(0, |id| self.exists(id, block)).await?;

        // Assert the boundary independently of the search's own bookkeeping:
        // `lo` must exist and `lo + 1` must not, or something is wrong (a
        // race, a bug in `find_boundary`) and a silently wrong population is
        // worse than a loud error.
        if !self.exists(lo, block).await? {
            anyhow::bail!(
                "highest_agent_id: boundary check failed — {lo} was reported to exist but does not"
            );
        }
        if self.exists(hi, block).await? {
            anyhow::bail!(
                "highest_agent_id: boundary check failed — {hi} was reported not to exist but does"
            );
        }

        Ok(Some(lo))
    }

    /// Every agent id that currently exists as of `block`, ascending.
    ///
    /// Walks `0..=highest_agent_id(block)` — a contiguous range, not a
    /// `Registered` event log. See [`highest_agent_id`](Self::highest_agent_id)
    /// for the contiguity assumption this relies on, how it was verified, and
    /// what would go wrong if it stopped holding. An empty registry
    /// (`highest_agent_id` returns `None`) yields an empty vec.
    pub async fn enumerate_agent_ids(&self, block: u64) -> Result<Vec<u64>> {
        match self.highest_agent_id(block).await? {
            None => Ok(Vec::new()),
            Some(max) => Ok((0..=max).collect()),
        }
    }

    /// Current owner and URI for one agent, both read AT `block`.
    pub async fn snapshot(&self, agent_id: u64, block: u64) -> Result<AgentSnapshot> {
        let c = IIdentityRegistry::new(self.address, &self.provider);
        let token_id = U256::from(agent_id);

        // `.call()` returns an `EthCall` builder (`IntoFuture`, not `Future`
        // directly) — wrapping it in an `async` block turns it into a plain
        // `Future` so it satisfies `retry_throttled`'s bound, exactly as
        // `.await`-ing it inline would have.
        let owner = retry_throttled(|| async {
            c.ownerOf(token_id).block(BlockId::from(block)).call().await
        })
        .await
        .with_context(|| format!("ownerOf({agent_id})"))?;
        let agent_uri = retry_throttled(|| async {
            c.tokenURI(token_id)
                .block(BlockId::from(block))
                .call()
                .await
        })
        .await
        .with_context(|| format!("tokenURI({agent_id})"))?;

        Ok(AgentSnapshot {
            agent_id,
            token_id,
            owner: format!("{owner:?}").to_lowercase(),
            agent_uri,
            block_number: block,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn a_genuine_error_is_not_treated_as_throttled() {
        assert!(!is_throttled(&"execution reverted"));
        assert!(!is_throttled(&"invalid address"));
    }

    /// The exact Alchemy 429 body observed sweeping Base in Task 8: "compute
    /// units per second capacity" exceeded. Must be recognised as throttling.
    #[test]
    fn recognises_alchemys_429_as_throttled() {
        let body = "HTTP error 429 with body: {\"jsonrpc\":\"2.0\",\"id\":232,\"error\":\
                    {\"code\":429,\"message\":\"Your app has exceeded its compute units \
                    per second capacity. If you have retries enabled, you can safely \
                    ignore this message.\"}}";
        assert!(is_throttled(&body));
        assert!(!is_revert(&body));
    }

    /// The exact shape observed calling `ownerOf` on a definitely-unminted id
    /// (999,999,999) against Base on 2026-07-27: JSON-RPC error code 3,
    /// message "execution reverted". Must be recognised as "does not exist".
    #[test]
    fn recognises_ownerof_revert_on_a_nonexistent_token() {
        let body = "server returned an error response: error code 3: execution reverted, \
                    data: \"0x7e273289000000000000000000000000000000000000000000000000000000003b9ac9ff\"";
        assert!(is_revert(&body));
        assert!(!is_throttled(&body));
    }

    #[test]
    fn a_timeout_is_not_treated_as_a_revert() {
        assert!(!is_revert(&"request timed out"));
        assert!(!is_revert(&"connection reset by peer"));
    }

    /// `retry_throttled` must retry a throttled failure until it succeeds,
    /// and must return the eventual success rather than an error. Uses the
    /// real production constants (`retry_throttled`, not `_with`) since two
    /// retries at a 1ms base is fast regardless.
    #[tokio::test]
    async fn retry_throttled_recovers_after_transient_429s() {
        let attempts = AtomicU32::new(0);
        let result: Result<&'static str, &'static str> = retry_throttled(|| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    Err("HTTP error 429: exceeded its compute units per second capacity")
                } else {
                    Ok("ok")
                }
            }
        })
        .await;
        assert_eq!(result, Ok("ok"));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    /// A genuine, non-throttling error must NOT be retried — it should return
    /// immediately on the first attempt, so a malformed request fails fast
    /// instead of being retried into a multi-minute timeout for no reason.
    #[tokio::test]
    async fn retry_throttled_does_not_retry_genuine_errors() {
        let attempts = AtomicU32::new(0);
        let result: Result<(), &'static str> = retry_throttled(|| {
            attempts.fetch_add(1, Ordering::SeqCst);
            async move { Err("execution reverted") }
        })
        .await;
        assert_eq!(result, Err("execution reverted"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    /// A throttled call that never recovers must give up after the retry
    /// budget, not spin forever. Uses `_with` at a 1ms backoff base so the
    /// test runs in milliseconds instead of the ~76s the production backoff
    /// constants would take for 8 retries.
    #[tokio::test]
    async fn retry_throttled_gives_up_after_the_retry_budget() {
        let attempts = AtomicU32::new(0);
        let max_retries = 3;
        let result: Result<(), &'static str> = retry_throttled_with(max_retries, 1, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async move { Err("HTTP error 429: rate limit exceeded") }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), max_retries + 1);
    }

    /// A population of exactly one (only id 0 exists) must not trigger any
    /// doubling — the very first probe at `hi = 1` should already be false.
    #[tokio::test]
    async fn find_boundary_a_single_existing_id() {
        let (lo, hi) = find_boundary(0, |id| async move { Ok(id == 0) })
            .await
            .unwrap();
        assert_eq!((lo, hi), (0, 1));
    }

    /// Mirrors the real, empirically-observed population: ids 0..=59997
    /// exist, 59998 does not. Exercises real doubling steps (1, 2, 4, ...,
    /// 65536) rather than a toy-sized fixture.
    #[tokio::test]
    async fn find_boundary_matches_the_observed_base_population() {
        let (lo, hi) = find_boundary(0, |id| async move { Ok(id <= 59_997) })
            .await
            .unwrap();
        assert_eq!((lo, hi), (59_997, 59_998));
    }

    /// A population whose size lands exactly on a power of two must not be
    /// off by one in either direction.
    #[tokio::test]
    async fn find_boundary_exact_power_of_two_boundary() {
        // ids 0..=63 exist (64 agents); 64 does not.
        let (lo, hi) = find_boundary(0, |id| async move { Ok(id <= 63) })
            .await
            .unwrap();
        assert_eq!((lo, hi), (63, 64));
    }

    /// A call that fails for a reason other than "does not exist" (the
    /// analogue of a throttled or malformed response reaching the search)
    /// must abort the search with an error rather than being interpreted
    /// either way.
    #[tokio::test]
    async fn find_boundary_propagates_a_genuine_error_instead_of_guessing() {
        let result = find_boundary(0, |id| async move {
            if id == 4 {
                anyhow::bail!("simulated transport error")
            } else {
                Ok(id <= 100)
            }
        })
        .await;
        assert!(
            result.is_err(),
            "a non-revert failure must not be treated as a boundary"
        );
    }

    /// Hits a real RPC endpoint, so it is `#[ignore]` by default:
    ///   RPC_URL_BASE=... cargo test -p chain -- --ignored --nocapture
    /// Prints what it read so a human can compare against a block explorer.
    #[tokio::test]
    #[ignore]
    async fn reads_current_owner_and_uri_from_base() {
        let rpc = std::env::var("RPC_URL_BASE").expect("RPC_URL_BASE");
        let reg = Registry::connect(&rpc, "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432")
            .await
            .unwrap();
        let block = reg.pinned_block().await.unwrap();

        // 1, 2, 3: sanity-check agents with plausible owners/URIs.
        // 1938, 1939, 1940, 1942, 1381: agent ids the indexer recorded with an
        // empty `agentURI` (`SELECT agent_id FROM agents WHERE domain = ''
        // LIMIT 5`). `tokenURI()` reads the same value independently of event
        // decoding, so comparing its result against the indexed empty string
        // settles whether that's real registry data or an indexer decode bug.
        for id in [1u64, 2, 3, 1938, 1939, 1940, 1942, 1381] {
            let s = reg.snapshot(id, block).await.unwrap();
            println!(
                "agent {} | owner {} | uri_len {} | uri {:?}",
                s.agent_id,
                s.owner,
                s.agent_uri.len(),
                s.agent_uri.chars().take(80).collect::<String>()
            );
            assert_eq!(s.agent_id, id);
            assert!(s.owner.starts_with("0x"), "owner must be hex");
            assert_eq!(s.owner, s.owner.to_lowercase(), "owner must be normalised");
        }
    }

    /// Hits a real RPC endpoint, so it is `#[ignore]` by default:
    ///   RPC_URL_BASE=... cargo test -p chain -- --ignored --nocapture
    /// Confirms the binary search actually finds the registry's real
    /// population and that its own boundary assertion is satisfied against
    /// live state, not just the fixtures in `find_boundary_*` above.
    #[tokio::test]
    #[ignore]
    async fn highest_agent_id_matches_base() {
        let rpc = std::env::var("RPC_URL_BASE").expect("RPC_URL_BASE");
        let reg = Registry::connect(&rpc, "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432")
            .await
            .unwrap();
        let block = reg.pinned_block().await.unwrap();
        let max = reg.highest_agent_id(block).await.unwrap();
        println!("highest_agent_id at block {block}: {max:?}");
        assert!(max.is_some());
        let max = max.unwrap();
        assert!(
            max >= 59_997,
            "population appears to have shrunk below a known floor"
        );
    }
}
