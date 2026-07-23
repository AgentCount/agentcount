//! The ingest loop — follow a chain block by block, forever.
//!
//! This is the beating heart of the indexer. For one chain it:
//!   1. asks the database where it left off (the *cursor*),
//!   2. fetches logs from the registry contracts for the next range of blocks,
//!   3. decodes each log into a [`RegistryEvent`],
//!   4. writes the results to Postgres and advances the cursor,
//!   5. sleeps briefly and repeats.
//!
//! ## Why a cursor, and why it's your safety net
//!
//! Long-running indexers crash — the RPC hiccups, the machine reboots, you
//! deploy. Persisting "last block I fully processed" means restarting simply
//! resumes from there instead of re-scanning the chain from genesis or, worse,
//! silently skipping blocks. Because the cursor lives in Postgres alongside the
//! events, a crash can never leave the two out of sync for long.
//!
//! ## A note on reorgs (chain reorganisations)
//!
//! The most recent blocks can be *reverted* by the network. A naive indexer that
//! trusts the chain tip will occasionally record events that later vanish. The
//! standard defence is to stay a few blocks behind the tip (a "confirmations"
//! buffer) so you only index blocks unlikely to be reorged. We flag where that
//! belongs below; implementing it properly is a great follow-up exercise.

use anyhow::Result;

use crate::chains::Chain;
use crate::store::Db;

/// How many blocks to fetch in one request. RPC providers cap the range and the
/// response size, so we page through history in chunks rather than asking for
/// millions of blocks at once.
const BLOCK_BATCH_SIZE: u64 = 2_000;

/// How many blocks to stay behind the chain tip, to dodge reorgs. Tune per
/// chain — Base and Ethereum have different reorg characteristics.
const CONFIRMATIONS: u64 = 5;

/// Run the ingest loop for one chain until the process is told to stop.
///
/// Takes the `Chain` *by value* (owned): this loop owns its provider for its
/// whole life. Takes the `Db` handle it needs to persist to. Returns `Result`
/// so a fatal error bubbles up to `main`, which decides whether to crash.
///
/// This function is `async` and effectively never returns under normal
/// operation — it's the top of an infinite `loop`. Because every `.await` inside
/// yields to the runtime, running two of these (one per chain) on one runtime is
/// cheap and they interleave naturally.
pub async fn run(chain: Chain, db: Db) -> Result<()> {
    // The `loop { ... }` keyword is Rust's infinite loop (nicer than `while
    // true`). We break out only on a fatal error via `?`.
    loop {
        // 1. Where did we get to last time? Falls back to a configured start
        //    block the first time we ever run for this chain+contract.
        //     let from_block = db.load_cursor(&chain.name).await?;

        // 2. What's the safe tip to index up to right now?
        //     let head = chain.provider.get_block_number().await?;
        //     let safe_head = head.saturating_sub(CONFIRMATIONS); // reorg buffer
        //     let to_block = (from_block + BLOCK_BATCH_SIZE).min(safe_head);
        //
        //    If we've already caught up, there's nothing to do — nap and re-loop.
        //     if from_block >= to_block {
        //         tokio::time::sleep(std::time::Duration::from_secs(12)).await;
        //         continue; // `continue` jumps back to the top of the loop
        //     }

        // 3. Fetch logs from all three registries in [from_block, to_block].
        //    You build an alloy `Filter` with the three registry addresses and
        //    the block range, then call `provider.get_logs(&filter).await?`.

        // 4. Decode + persist. `decode_log` returns `Option`, so we skip logs we
        //    don't recognise. `filter_map` keeps only the `Some(..)` results —
        //    a very common iterator idiom worth learning:
        //
        //     let events: Vec<_> = raw_logs
        //         .iter()
        //         .filter_map(crate::bindings::decode_log)
        //         .collect();
        //
        //    Persist the raw logs (audit trail) AND the decoded events, then move
        //    the cursor to `to_block` — ideally in ONE transaction so a crash
        //    can't advance the cursor past events it didn't actually save:
        //
        //     db.write_batch(&chain.name, &raw_logs, &events, to_block).await?;

        let _ = (&chain, &db, BLOCK_BATCH_SIZE, CONFIRMATIONS);
        todo!("implement one iteration: cursor → fetch logs → decode → persist → advance");
    }
}
