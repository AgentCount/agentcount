//! Reading the Identity Registry: which agents exist, and what the chain says
//! about each one RIGHT NOW.
//!
//! Rust concept spotlight: **`sol!` generates call types, not just events.**
//! Given a function signature it produces a typed builder, so `ownerOf(id)`
//! is a compile-checked call rather than hand-packed calldata.

use alloy::eips::BlockId;
use alloy::primitives::{Address, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::rpc::types::Filter;
use alloy::sol;
use alloy::sol_types::SolEvent;
use anyhow::{Context, Result};

sol! {
    #[sol(rpc)]
    interface IIdentityRegistry {
        function ownerOf(uint256 tokenId) external view returns (address);
        function tokenURI(uint256 tokenId) external view returns (string);
    }

    // Discovery only — the state comes from the calls above.
    event Registered(uint256 indexed agentId, string agentURI, address indexed owner);
}

/// Default blocks per `getLogs` call when discovering agent ids. Deliberately
/// modest: Alchemy's free tier caps a `getLogs` block range at 10, and other
/// providers reject wide ranges outright by result size. Override with
/// `CHAIN_BLOCK_BATCH` without recompiling. Mirrors
/// `crates/indexer/src/ingest.rs::DEFAULT_BLOCK_BATCH_SIZE` / `INDEXER_BLOCK_BATCH`.
const DEFAULT_BLOCK_BATCH_SIZE: u64 = 500;

fn block_batch_size() -> u64 {
    std::env::var("CHAIN_BLOCK_BATCH")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_BLOCK_BATCH_SIZE)
}

/// Does this RPC error mean "your block range is too large / returned too many
/// results"? Providers cap `getLogs` by result size (Base public RPC: "backend
/// response too large"; Alchemy: "query returned more than 10000 results") or
/// by block-range width outright (Alchemy free tier: "up to a 10 block
/// range"), and the only fix is a narrower range — retrying the same one never
/// helps. A timeout is NOT this (splitting won't beat a provider that
/// throttles `getLogs` wholesale), so those strings are deliberately excluded.
///
/// Identical matching to `crates/indexer/src/ingest.rs::is_range_too_large`,
/// which solved this same problem for the ingest loop first — reused here
/// rather than reinvented, per that module's doc comment.
fn is_range_too_large(e: &impl std::fmt::Display) -> bool {
    let s = e.to_string().to_lowercase();
    s.contains("too large")
        || s.contains("too many results")
        || s.contains("response size")
        || s.contains("limited to")
        || s.contains("exceed")
        || s.contains("block range")
}

/// Split `[from, to]` into inclusive, non-overlapping, ascending chunks of at
/// most `batch` blocks each. Pure and network-free, so it's the part of the
/// chunking strategy this crate can actually unit-test — the adaptive-split
/// half (`is_range_too_large`) only shows its behaviour against a real
/// provider, exercised in Task 8's sweep instead.
fn chunk_ranges(from: u64, to: u64, batch: u64) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    if from > to || batch == 0 {
        return out;
    }
    let mut lo = from;
    loop {
        let hi = lo.saturating_add(batch - 1).min(to);
        out.push((lo, hi));
        if hi == to {
            break;
        }
        lo = hi + 1;
    }
    out
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

    /// Every agent id ever registered, from `Registered` logs.
    ///
    /// A single `eth_getLogs` over the whole deploy-block-to-head range is not
    /// viable: every provider caps it somehow, and Alchemy's free tier caps
    /// the *block* range itself at 10 — the deploy-to-head span on Base is on
    /// the order of 250,000 blocks. So this chunks the range into
    /// `block_batch_size()`-sized pieces up front, then adaptively halves any
    /// piece the provider still rejects. Same two-part approach as
    /// `crates/indexer/src/ingest.rs::fetch_logs` (configurable batch size +
    /// adaptive splitting on "too large" errors) — that module solved this
    /// exact problem for the ingest loop and `is_range_too_large` below is the
    /// same string matching, not a reinvention.
    pub async fn enumerate_agent_ids(&self, from_block: u64, to_block: u64) -> Result<Vec<u64>> {
        // Seed a work stack with fixed-size chunks so a well-behaved provider
        // never even sees a range wider than `batch`. Order doesn't matter —
        // ids are sorted and deduped at the end regardless of arrival order.
        let mut stack: Vec<(u64, u64)> = chunk_ranges(from_block, to_block, block_batch_size());
        let mut ids: Vec<u64> = Vec::new();
        while let Some((lo, hi)) = stack.pop() {
            let filter = Filter::new()
                .address(self.address)
                .event_signature(Registered::SIGNATURE_HASH)
                .from_block(lo)
                .to_block(hi);
            match self.provider.get_logs(&filter).await {
                Ok(logs) => ids.extend(
                    logs.iter()
                        .filter_map(|l| l.log_decode::<Registered>().ok())
                        // `log_decode` returns the RPC `Log<Registered>` wrapper
                        // (block/tx metadata plus the decoded event). `.data()`
                        // reaches the decoded event itself — the plain field
                        // access `d.agentId` (skipping `.data()`) does not
                        // compile against this alloy version.
                        .map(|d| d.data().agentId.to::<u64>()),
                ),
                // A range still too large for this provider: halve it and
                // retry both halves. Never retry the SAME range unchanged —
                // that only wastes a round trip against a doomed request.
                Err(e) if is_range_too_large(&e) && lo < hi => {
                    let mid = lo + (hi - lo) / 2;
                    stack.push((mid + 1, hi));
                    stack.push((lo, mid));
                }
                Err(e) => return Err(e).context("get_logs"),
            }
        }

        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    /// Current owner and URI for one agent, both read AT `block`.
    pub async fn snapshot(&self, agent_id: u64, block: u64) -> Result<AgentSnapshot> {
        let c = IIdentityRegistry::new(self.address, &self.provider);
        let token_id = U256::from(agent_id);

        let owner = c
            .ownerOf(token_id)
            .block(BlockId::from(block))
            .call()
            .await
            .with_context(|| format!("ownerOf({agent_id})"))?;
        let agent_uri = c
            .tokenURI(token_id)
            .block(BlockId::from(block))
            .call()
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

    /// A ~250,000-block sweep (Base's actual deploy-to-head span) at the
    /// default batch size must produce many bounded chunks, none wider than
    /// the batch, that together cover the range with no gap and no overlap.
    #[test]
    fn chunk_ranges_covers_a_wide_span_with_no_gaps_or_overlaps() {
        let chunks = chunk_ranges(41_663_783, 41_663_783 + 250_000, 500);
        assert!(chunks.len() > 400, "expected many chunks, got {}", chunks.len());
        for (lo, hi) in &chunks {
            assert!(hi - lo < 500, "chunk {lo}..{hi} exceeds the batch size");
        }
        assert_eq!(chunks.first().unwrap().0, 41_663_783);
        assert_eq!(chunks.last().unwrap().1, 41_663_783 + 250_000);
        for w in chunks.windows(2) {
            assert_eq!(w[0].1 + 1, w[1].0, "gap or overlap between {:?} and {:?}", w[0], w[1]);
        }
    }

    /// A range narrower than the batch size is a single chunk, not padded out
    /// past `to_block` — a smoke-test sweep over a handful of blocks must not
    /// silently widen the range it was asked to enumerate.
    #[test]
    fn chunk_ranges_a_narrow_span_is_one_chunk() {
        assert_eq!(chunk_ranges(100, 105, 500), vec![(100, 105)]);
    }

    /// `from > to` (an empty window) yields no chunks and, by extension, no
    /// `get_logs` call at all — never an inverted or wraparound range.
    #[test]
    fn chunk_ranges_empty_window_yields_nothing() {
        assert_eq!(chunk_ranges(105, 100, 500), Vec::<(u64, u64)>::new());
    }

    #[test]
    fn chunk_ranges_exact_multiple_has_no_trailing_empty_chunk() {
        // 1000 blocks at batch 500 must be exactly two chunks, not three.
        let chunks = chunk_ranges(0, 999, 500);
        assert_eq!(chunks, vec![(0, 499), (500, 999)]);
    }

    /// Alchemy's free tier rejects on block-range width specifically, phrased
    /// differently from the result-size caps other providers use. Both must
    /// be recognised as "narrow the range and retry", never as a bare error.
    #[test]
    fn recognises_alchemys_block_range_cap_as_too_large() {
        assert!(is_range_too_large(&"eth_getLogs is limited to a 10 block range"));
    }

    #[test]
    fn recognises_result_size_caps_as_too_large() {
        assert!(is_range_too_large(&"query returned too many results"));
        assert!(is_range_too_large(&"backend response too large"));
    }

    /// A timeout is a different failure: splitting the range doesn't help a
    /// provider that's throttling `getLogs` wholesale, so it must NOT be
    /// treated as "too large" and endlessly re-split.
    #[test]
    fn a_timeout_is_not_treated_as_too_large() {
        assert!(!is_range_too_large(&"request timed out"));
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
}
