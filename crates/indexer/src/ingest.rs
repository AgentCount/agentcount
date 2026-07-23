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
//! The most recent blocks can be reverted by the network. We stay a few blocks
//! behind the tip (the `CONFIRMATIONS` buffer) so we only index blocks unlikely
//! to be reorged. Deeper reorg handling (rolling back on a detected revert) is a
//! good follow-up exercise.

use alloy::providers::Provider;
use alloy::rpc::types::Filter;
use anyhow::Result;

use crate::bindings::{self, IndexedLog};
use crate::chains::Chain;
use crate::store::Db;

/// Fetch at most this many blocks per request; RPC providers cap the range.
const BLOCK_BATCH_SIZE: u64 = 2_000;

/// How many blocks to stay behind the tip, to dodge reorgs.
const CONFIRMATIONS: u64 = 5;

/// Run the ingest loop for one chain until the process stops.
///
/// Takes the `Chain` by value (this loop owns its provider for life) and a `Db`
/// handle to persist through. `async` and effectively infinite — every `.await`
/// yields to the runtime, so running one of these per chain interleaves cheaply.
pub async fn run(chain: Chain, db: Db) -> Result<()> {
    loop {
        // 1. Where did we get to last time?
        let from_block = db.load_cursor(&chain.name).await?;

        // 2. What's the safe tip to index up to right now?
        let head = chain.provider.get_block_number().await?;
        let safe_head = head.saturating_sub(CONFIRMATIONS);

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
            .filter_map(|log| bindings::index_log(&chain.name, log))
            .collect();

        // 5. Persist raw logs + decoded rows + the new cursor, atomically.
        db.write_batch(&chain.name, &indexed, to_block).await?;

        tracing::info!(
            chain = %chain.name,
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
