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
    pub async fn enumerate_agent_ids(&self, from_block: u64, to_block: u64) -> Result<Vec<u64>> {
        let filter = Filter::new()
            .address(self.address)
            .event_signature(Registered::SIGNATURE_HASH)
            .from_block(from_block)
            .to_block(to_block);
        let logs = self.provider.get_logs(&filter).await.context("get_logs")?;
        let mut ids: Vec<u64> = logs
            .iter()
            .filter_map(|l| l.log_decode::<Registered>().ok())
            // `log_decode` returns the RPC `Log<Registered>` wrapper (block/tx
            // metadata plus the decoded event). `.data()` reaches the decoded
            // event itself — the plain field access `d.agentId` (skipping
            // `.data()`) does not compile against this alloy version.
            .map(|d| d.data().agentId.to::<u64>())
            .collect();
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
