//! The ingest loop — follow a chain block by block, forever.
//!
//! For one chain it: asks the DB where it left off (the *cursor*), fetches logs
//! from the registry contracts for the next range of blocks, decodes them, writes
//! the results, advances the cursor, sleeps, and repeats.
//!
//! ## Why a cursor, and why it's your safety net
//!
//! Long-running indexers crash — the RPC hiccups, you deploy, the box reboots.
//! Persisting "last block I fully processed" means a restart resumes from there
//! instead of rescanning from genesis or silently skipping blocks. Because the
//! cursor and the events are written in the SAME transaction, they can never
//! drift apart.
//!
//! ## Reorgs (chain reorganisations)
//!
//! The most recent blocks can be reverted by the network. We stay a per-chain
//! `confirmations` buffer behind the tip (fast chains need more blocks for the
//! same wall-clock safety), and every raw event stores its block hash so a
//! deeper reorg is at least *detectable* and re-processable from the audit log.

use alloy::providers::Provider;
use alloy::rpc::types::Filter;
use anyhow::Result;

use crate::bindings::{self, IndexedLog};
use crate::chains::Chain;
use crate::store::Db;

/// Fetch at most this many blocks per request; RPC providers cap the range.
const BLOCK_BATCH_SIZE: u64 = 2_000;

/// Where to resume: the block AFTER the cursor, or the registry deploy block
/// on a chain we've never indexed. Pure, so it's trivially unit-testable —
/// off-by-one bugs here mean skipped or double-fetched blocks.
pub fn resume_from(cursor: Option<i64>, deploy_block: i64) -> u64 {
    match cursor {
        Some(last) => (last + 1) as u64,
        None => deploy_block.max(0) as u64,
    }
}

/// Run the ingest loop for one chain until the process stops.
///
/// Takes the `Chain` by value (this loop owns its provider for life) and a `Db`
/// handle to persist through. `async` and effectively infinite — every `.await`
/// yields to the runtime, so running one of these per chain interleaves cheaply.
pub async fn run(chain: Chain, db: Db) -> Result<()> {
    let name = chain.config.chain.clone();
    let confirmations = chain.config.confirmations as u64;
    loop {
        // 1. Where did we get to last time?
        let from_block = resume_from(db.load_cursor(&name).await?, chain.config.deploy_block);

        // 2. What's the safe tip to index up to right now?
        let head = chain.provider.get_block_number().await?;
        let safe_head = head.saturating_sub(confirmations);

        // Already caught up? Nap, then re-loop. `continue` jumps to the top.
        if from_block > safe_head {
            tokio::time::sleep(std::time::Duration::from_secs(12)).await;
            continue;
        }

        let to_block = (from_block + BLOCK_BATCH_SIZE).min(safe_head);

        // 3. Fetch logs from all registries in [from_block, to_block].
        let filter = Filter::new()
            .address(chain.registries.clone())
            .from_block(from_block)
            .to_block(to_block);
        let logs = chain.provider.get_logs(&filter).await?;

        // 4. Decode. `filter_map` keeps only the logs we recognise (the `Some`s),
        //    discarding the rest — a very common iterator idiom.
        let indexed: Vec<IndexedLog> = logs
            .iter()
            .filter_map(|log| bindings::index_log(&name, log))
            .collect();

        // 5. Persist raw logs + decoded rows + the new cursor, atomically.
        db.write_batch(&name, &indexed, to_block).await?;

        tracing::info!(
            chain = %name,
            from = from_block,
            to = to_block,
            events = indexed.len(),
            "indexed block range"
        );

        // If we've reached the safe tip, wait for new blocks before looping.
        if to_block >= safe_head {
            tokio::time::sleep(std::time::Duration::from_secs(12)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cursor stores the last FULLY PROCESSED block; resumption starts at
    /// the NEXT one. The old code re-fetched the cursor block every loop.
    #[test]
    fn resume_starts_after_the_cursor() {
        assert_eq!(resume_from(Some(100), 50), 101);
    }

    /// First run (no cursor row yet): start at the registry's deploy block,
    /// never at genesis.
    #[test]
    fn first_run_starts_at_deploy_block() {
        assert_eq!(resume_from(None, 34_567_890), 34_567_890);
    }
}
