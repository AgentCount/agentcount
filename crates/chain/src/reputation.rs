//! Reading the Reputation Registry: who has left feedback for an agent, and
//! how much of it there is — the raw material rung 7 (`independent`) in
//! `crates/checks` uses to ask "does anyone other than the owner vouch for
//! this agent?"
//!
//! ## `getSummary`'s empty-`clientAddresses` question, settled
//!
//! The task brief this module was written from assumed calling `getSummary`
//! with an empty `clientAddresses` array means "all feedback, unfiltered".
//! That is not what the pinned spec says, and it is not what the deployed
//! contract does — both were checked, not assumed:
//!
//! - **Spec** (`spec/ERC8004SPEC.md` lines 323-325): "`clientAddresses` MUST
//!   be provided (non-empty); results without filtering by clientAddresses
//!   are subject to Sybil/spam attacks." The spec does not offer an
//!   empty-array sentinel for "everyone" — it requires an explicit,
//!   non-empty list.
//! - **Live contract**, Base, `0x8004baa17c55a88189ae136b182e5fda19de9b63`,
//!   agent id 3, block pinned at call time: `getSummary(3, [], "", "")`
//!   reverts with `clientAddresses required` (Solidity custom revert reason,
//!   observed verbatim via `cast call`). It does not return zero, it does
//!   not return "all feedback" — it refuses the call outright.
//!
//! So "all feedback" is computed the only way the contract actually
//! supports it: call [`IReputationRegistry::getClients`] first to enumerate
//! every address that has ever left feedback, then pass that exact
//! (non-empty) list back into `getSummary` as the filter. That is not an
//! approximation of "everyone" — for this agent, at this block, it *is*
//! everyone, because `getClients` is itself the registry's own record of who
//! that is. An agent with zero clients never reaches `getSummary` at all
//! (see [`Reputation::feedback`]): calling it with `[]` would revert, and
//! there is nothing to summarise regardless.

use alloy::eips::BlockId;
use alloy::primitives::{Address, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::sol;
use anyhow::{Context, Result};

sol! {
    #[sol(rpc)]
    interface IReputationRegistry {
        function getClients(uint256 agentId) external view returns (address[] memory);
        function getSummary(uint256 agentId, address[] calldata clientAddresses, string tag1, string tag2)
            external view returns (uint64 count, int128 summaryValue, uint8 summaryValueDecimals);
    }
}

/// Same throttling signature Alchemy returns everywhere else in this crate —
/// see `registry.rs`'s `is_throttled` for the verbatim observed body. Kept as
/// a private copy rather than shared so this module has no dependency on
/// `registry`'s internals; the two are allowed to drift if the failure
/// signatures ever do.
fn is_throttled(e: &impl std::fmt::Display) -> bool {
    let s = e.to_string().to_lowercase();
    s.contains("429")
        || s.contains("compute units per second")
        || s.contains("too many requests")
        || s.contains("rate limit")
}

const MAX_THROTTLE_RETRIES: u32 = 8;
const THROTTLE_BACKOFF_BASE_MS: u64 = 300;

/// Identical retry shape to `registry.rs::retry_throttled` — duplicated
/// rather than shared across a two-file crate to keep each module
/// independently readable; see that module's doc comments for the full
/// rationale of what gets retried and why.
async fn retry_throttled<T, E, F, Fut>(f: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempt = 0u32;
    let mut f = f;
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if is_throttled(&e) && attempt < MAX_THROTTLE_RETRIES => {
                let backoff_ms = THROTTLE_BACKOFF_BASE_MS * (1u64 << attempt);
                tracing::warn!(
                    "throttled (attempt {}/{MAX_THROTTLE_RETRIES}), backing off {backoff_ms}ms: {e:#}",
                    attempt + 1
                );
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// What the Reputation Registry says about one agent's feedback, at one
/// block.
#[derive(Debug, Clone)]
pub struct FeedbackReads {
    /// Every address that has ever left feedback for this agent, lowercase
    /// hex. Normalised here — once, at the read — so nothing downstream
    /// (rung 7's owner comparison) has to remember to.
    pub clients: Vec<String>,
    /// Total feedback entry count across all `clients`, from `getSummary`.
    /// `0` when `clients` is empty — `getSummary` is never called in that
    /// case (it would revert; see the module doc comment).
    pub feedback_count: u64,
}

pub struct Reputation {
    provider: DynProvider,
    address: Address,
}

impl Reputation {
    pub async fn connect(rpc_url: &str, address: &str) -> Result<Self> {
        let provider = ProviderBuilder::new()
            .connect(rpc_url)
            .await
            .with_context(|| format!("connecting to {rpc_url}"))?
            .erased();
        let address: Address = address
            .parse()
            .context("parsing reputation registry address")?;
        Ok(Self { provider, address })
    }

    /// The block every read in this run is pinned to — same role as
    /// `Registry::pinned_block` in `registry.rs`.
    pub async fn pinned_block(&self) -> Result<u64> {
        Ok(self.provider.get_block_number().await?)
    }

    /// Every client address and the total feedback count for `agent_id`, as
    /// of `block`. Pinned to the same block as every other read in the run —
    /// see `registry.rs`'s module doc for why an unpinned sweep would census
    /// a population that never simultaneously existed.
    ///
    /// `getClients` is read first. If it comes back empty, `getSummary` is
    /// not called at all: the contract requires a non-empty
    /// `clientAddresses` and reverts otherwise (verified live — see the
    /// module doc comment), and there is nothing to summarise for an agent
    /// with no clients regardless.
    pub async fn feedback(&self, agent_id: u64, block: u64) -> Result<FeedbackReads> {
        let c = IReputationRegistry::new(self.address, &self.provider);
        let token_id = U256::from(agent_id);

        let clients: Vec<Address> = retry_throttled(|| async {
            c.getClients(token_id)
                .block(BlockId::from(block))
                .call()
                .await
        })
        .await
        .with_context(|| format!("getClients({agent_id})"))?;

        if clients.is_empty() {
            return Ok(FeedbackReads {
                clients: Vec::new(),
                feedback_count: 0,
            });
        }

        let summary = retry_throttled(|| async {
            c.getSummary(token_id, clients.clone(), String::new(), String::new())
                .block(BlockId::from(block))
                .call()
                .await
        })
        .await
        .with_context(|| format!("getSummary({agent_id}, {} clients)", clients.len()))?;

        Ok(FeedbackReads {
            clients: clients
                .into_iter()
                .map(|a| format!("{a:?}").to_lowercase())
                .collect(),
            feedback_count: summary.count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hits a real RPC endpoint, so it is `#[ignore]` by default:
    ///   RPC_URL_BASE=... cargo test -p chain -- --ignored --nocapture
    ///
    /// Agent id 3 on Base is known (via `getClients`, checked live before
    /// writing this test) to have real feedback: 9 distinct client
    /// addresses. Prints what it read so a human can compare against a
    /// block explorer.
    #[tokio::test]
    #[ignore]
    async fn reads_feedback_for_an_agent_with_real_activity_on_base() {
        let rpc = std::env::var("RPC_URL_BASE").expect("RPC_URL_BASE");
        let rep = Reputation::connect(&rpc, "0x8004baa17c55a88189ae136b182e5fda19de9b63")
            .await
            .unwrap();
        let block = rep.pinned_block().await.unwrap();

        let reads = rep.feedback(3, block).await.unwrap();
        println!(
            "agent 3 at block {block} | feedback_count {} | distinct clients {} | {:?}",
            reads.feedback_count,
            reads.clients.len(),
            reads.clients
        );
        assert!(
            reads.feedback_count > 0,
            "agent 3 is known to have feedback"
        );
        assert!(!reads.clients.is_empty());
        for c in &reads.clients {
            assert!(c.starts_with("0x"));
            assert_eq!(c, &c.to_lowercase(), "client address must be normalised");
        }
    }

    /// Hits a real RPC endpoint, so it is `#[ignore]` by default. Confirms —
    /// independently of the module doc comment's claim — that calling
    /// `getSummary` with an empty `clientAddresses` array reverts rather
    /// than returning "all feedback". This is the exact check the task
    /// brief asked for before trusting the "empty array means everyone"
    /// assumption.
    #[tokio::test]
    #[ignore]
    async fn empty_client_list_is_rejected_by_get_summary_not_treated_as_everyone() {
        let rpc = std::env::var("RPC_URL_BASE").expect("RPC_URL_BASE");
        let rep = Reputation::connect(&rpc, "0x8004baa17c55a88189ae136b182e5fda19de9b63")
            .await
            .unwrap();
        let block = rep.pinned_block().await.unwrap();
        let c = IReputationRegistry::new(rep.address, &rep.provider);
        let result = c
            .getSummary(U256::from(3u64), vec![], String::new(), String::new())
            .block(BlockId::from(block))
            .call()
            .await;
        let err = match result {
            Ok(_) => panic!("empty clientAddresses must revert, not return a summary"),
            Err(e) => e,
        };
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("clientaddresses") || msg.contains("execution reverted"),
            "unexpected error shape: {msg}"
        );
    }
}
