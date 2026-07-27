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
//! ## Transient errors don't kill the process
//!
//! RPC providers time out, rate-limit, and blip — especially on free tiers. A
//! single failed request must NOT bring the indexer down. Each pass through
//! `run_batch` is fallible; the outer loop catches errors, backs off, and
//! retries the SAME range. That's safe precisely because the cursor only moves
//! on success — a failed batch changes nothing, so re-running it is idempotent.
//!
//! ## Reorgs (chain reorganisations)
//!
//! We stay a per-chain `confirmations` buffer behind the tip, and every raw
//! event stores its block hash, so a deeper reorg is at least detectable and
//! re-processable from the audit log.

use std::time::Duration;

use alloy::providers::Provider;
use alloy::rpc::types::Filter;
use anyhow::{Context, Result};

use crate::bindings::{self, IndexedLog};
use crate::chains::Chain;
use crate::store::Db;

/// Default blocks per `getLogs`. Deliberately modest: a large range makes the
/// provider do more work per request, and busy chains on free-tier RPC time out
/// on wide ranges. Override with `INDEXER_BLOCK_BATCH` without recompiling.
const DEFAULT_BLOCK_BATCH_SIZE: u64 = 500;

/// Longest we back off between failed batches (exponential up to this).
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Read the batch size from the environment once, falling back to the default.
fn block_batch_size() -> u64 {
    std::env::var("INDEXER_BLOCK_BATCH")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_BLOCK_BATCH_SIZE)
}

/// Where to resume: the block AFTER the cursor, or the registry deploy block
/// on a chain we've never indexed. Pure, so it's trivially unit-testable —
/// off-by-one bugs here mean skipped or double-fetched blocks.
pub fn resume_from(cursor: Option<i64>, deploy_block: i64) -> u64 {
    match cursor {
        Some(last) => (last + 1) as u64,
        None => deploy_block.max(0) as u64,
    }
}

/// Does this RPC error mean "your block range returned too many logs"? Providers
/// cap `getLogs` by result size (Base public RPC: "backend response too large";
/// Alchemy: "query returned more than 10000 results"), and the only fix is a
/// narrower range — retrying the same one never helps. A timeout is NOT this
/// (splitting won't beat a provider that throttles getLogs wholesale), so those
/// strings are deliberately excluded.
fn is_range_too_large(e: &impl std::fmt::Display) -> bool {
    let s = e.to_string().to_lowercase();
    s.contains("too large")
        || s.contains("too many results")
        || s.contains("response size")
        || s.contains("limited to")
        || s.contains("exceed")
        // Providers that cap the *block* range (e.g. Alchemy free: "up to a 10
        // block range"). Matching this makes the indexer split down to a
        // workable range instead of retrying the same doomed one forever.
        || s.contains("block range")
}

/// Fetch all logs in `[from, to]`, transparently halving the range whenever the
/// provider rejects it as too large. A work stack (not recursion — async
/// recursion needs boxing) drains sub-ranges until each fits. Order of the
/// returned logs doesn't matter: every consumer keys off block/tx, not vec
/// position, and the cursor still advances to `to` regardless of how we split.
async fn fetch_logs(chain: &Chain, from: u64, to: u64) -> Result<Vec<alloy::rpc::types::Log>> {
    let mut out = Vec::new();
    let mut stack = vec![(from, to)];
    while let Some((lo, hi)) = stack.pop() {
        let filter = Filter::new()
            .address(chain.registries.clone())
            .from_block(lo)
            .to_block(hi);
        match chain.provider.get_logs(&filter).await {
            Ok(mut logs) => out.append(&mut logs),
            Err(e) if is_range_too_large(&e) && lo < hi => {
                let mid = lo + (hi - lo) / 2;
                stack.push((mid + 1, hi));
                stack.push((lo, mid));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(out)
}

/// What one batch attempt accomplished — used to decide whether to nap.
enum Progress {
    /// Caught up to the safe tip; wait for new blocks.
    Idle,
    /// Processed a range; loop again immediately to keep backfilling.
    Advanced,
}

/// Run the ingest loop for one chain until the process stops. Never returns on
/// transient errors — it logs, backs off, and retries the same range.
pub async fn run(chain: Chain, db: Db) -> Result<()> {
    let name = chain.config.chain.clone();
    let batch = block_batch_size();
    tracing::info!(chain = %name, batch, "ingest loop starting");

    let mut backoff = Duration::from_secs(1);
    loop {
        match run_batch(&chain, &db, &name, batch).await {
            Ok(Progress::Idle) => {
                backoff = Duration::from_secs(1);
                tokio::time::sleep(Duration::from_secs(12)).await;
            }
            Ok(Progress::Advanced) => {
                backoff = Duration::from_secs(1);
            }
            Err(e) => {
                // Transient by assumption: log the cause, wait, retry the same
                // range. The cursor hasn't moved, so nothing is lost or doubled.
                tracing::warn!(chain = %name, "batch failed: {e:#}; retrying in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
}

/// Process one batch: resume point → safe tip → fetch → decode → persist.
/// Fallible; the caller decides how to react to an error.
async fn run_batch(chain: &Chain, db: &Db, name: &str, batch: u64) -> Result<Progress> {
    // 1. Where did we get to, and how far is it safe to index right now?
    let from_block = resume_from(db.load_cursor(name).await?, chain.config.deploy_block);
    let head = chain.provider.get_block_number().await?;
    let safe_head = head.saturating_sub(chain.config.confirmations as u64);

    if from_block > safe_head {
        return Ok(Progress::Idle);
    }

    let to_block = (from_block + batch).min(safe_head);

    // 2. Fetch logs from all registries in [from_block, to_block], splitting
    //    the range if the provider says the response is too big.
    let logs = fetch_logs(chain, from_block, to_block).await?;

    // 3. Fetch the real header for every distinct block that produced a log.
    //    eth_getLogs doesn't reliably include timestamps; the header is the
    //    ground truth, and registered_at must be block time, never wall-clock.
    //    One RPC per distinct block, cached across this batch.
    let mut headers: std::collections::HashMap<u64, (chrono::DateTime<chrono::Utc>, String)> =
        std::collections::HashMap::new();
    for number in logs.iter().filter_map(|l| l.block_number) {
        if headers.contains_key(&number) {
            continue;
        }
        let block = chain
            .provider
            .get_block_by_number(number.into())
            .await?
            .with_context(|| format!("{name}: block {number} not found for its own logs"))?;
        let ts = chrono::DateTime::from_timestamp(block.header.timestamp as i64, 0)
            .with_context(|| format!("{name}: block {number} has invalid timestamp"))?;
        headers.insert(number, (ts, block.header.hash.to_string().to_lowercase()));
    }

    // 4. Decode. `filter_map` keeps only the logs we recognise (the `Some`s).
    let indexed: Vec<IndexedLog> = logs
        .iter()
        .filter_map(|log| {
            let number = log.block_number?;
            let (ts, hash) = headers.get(&number)?;
            bindings::index_log(name, log, *ts, hash)
        })
        .collect();

    // 5. Persist raw logs + decoded rows + the new cursor, atomically.
    db.write_batch(name, &indexed, to_block).await?;

    tracing::info!(
        chain = %name,
        from = from_block,
        to = to_block,
        events = indexed.len(),
        "indexed block range"
    );

    Ok(Progress::Advanced)
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
