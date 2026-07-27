//! # sweeper — run the conformance ladder over one chain, once.
//!
//! A run is the unit of work and the unit of citation: it pins a block, reads
//! every agent's current state, answers the rungs it can, and writes both the
//! database rows and the `data/<run_id>/` export. Runs are immutable; to get
//! newer answers you take a new run, never edit an old one.
//!
//! Day 1 answers rung 1 only. Rungs 2-7 are ABSENT from the output rather than
//! reported as `skipped` — "we did not ask" and "we could not ask" are
//! different claims and the schema keeps them different.

mod export;
mod store;

use anyhow::{Context, Result};
use chrono::Utc;
use futures::stream::{self, StreamExt};
use uuid::Uuid;

/// How many `ownerOf`/`tokenURI` pairs to read at once. Conservative: a public
/// RPC endpoint is a shared resource and this is not a race. Lowered from 8
/// after Task 8's first live sweep hit Alchemy's free-tier "compute units per
/// second" cap immediately — override with `RPC_CONCURRENCY` without
/// recompiling, same pattern as `CHAIN_BLOCK_BATCH` in `crates/chain`.
const DEFAULT_RPC_CONCURRENCY: usize = 3;

fn rpc_concurrency() -> usize {
    std::env::var("RPC_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_RPC_CONCURRENCY)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let chain_name = std::env::args().nth(1).unwrap_or_else(|| "base".to_string());
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let rpc_var = format!("RPC_URL_{}", chain_name.to_uppercase());
    let rpc_url = std::env::var(&rpc_var).with_context(|| format!("{rpc_var} must be set"))?;

    let db = store::Db::connect(&database_url).await?;
    let (chain_id, registry_addr, deploy_block) = db.chain_config(&chain_name).await?;

    let registry = chain::Registry::connect(&rpc_url, &registry_addr).await?;
    let pinned = registry.pinned_block().await?;
    tracing::info!("sweeping {chain_name} at block {pinned}");

    let run_id = Uuid::new_v4();
    let checker_commit = env!("CHECKER_COMMIT");
    let rerun = format!("cargo run -p sweeper -- {chain_name}   # at block {pinned}");

    db.open_run(&store::RunMeta {
        run_id,
        chain: chain_name.clone(),
        schema_version: checks::SCHEMA_VERSION,
        checker_version: checks::CHECKER_VERSION.to_string(),
        checker_commit: checker_commit.to_string(),
        spec_commit: checks::SPEC_COMMIT.to_string(),
        rerun_command: rerun.clone(),
    })
    .await?;

    let ids = registry
        .enumerate_agent_ids(deploy_block as u64, pinned)
        .await?;
    tracing::info!("{} agent ids discovered", ids.len());

    // Read current state for each id, bounded. `buffer_unordered` keeps at most
    // RPC_CONCURRENCY reads in flight; results arrive out of order, which is
    // fine because each carries its own agent_id.
    let snapshots: Vec<chain::AgentSnapshot> = stream::iter(ids)
        .map(|id| {
            let registry = &registry;
            async move {
                match registry.snapshot(id, pinned).await {
                    Ok(s) => Some(s),
                    Err(e) => {
                        // An RPC failure is OUR problem, not the agent's: skip
                        // it from this run rather than recording a `fail`.
                        tracing::warn!("snapshot({id}) failed: {e:#}");
                        None
                    }
                }
            }
        })
        .buffer_unordered(rpc_concurrency())
        .filter_map(|o| async move { o })
        .collect()
        .await;

    tracing::info!("{} snapshots read", snapshots.len());

    export::write_manifest(&export::RunManifest {
        run_id: run_id.to_string(),
        chain: &chain_name,
        chain_id: chain_id as u64,
        registry: &registry_addr,
        pinned_block: pinned,
        started_at: Utc::now().to_rfc3339(),
        schema_version: checks::SCHEMA_VERSION,
        checker_version: checks::CHECKER_VERSION,
        checker_commit,
        spec_commit: checks::SPEC_COMMIT,
        rerun_command: &rerun,
        agent_count: snapshots.len(),
    })?;

    for s in &snapshots {
        let now = Utc::now();
        let rung1 = checks::registered(
            &checks::RegisteredInput {
                chain_id: chain_id as u64,
                registry: registry_addr.clone(),
                token_id: s.token_id.to_string(),
                owner: s.owner.clone(),
                block_number: s.block_number,
                // The registration tx lives in raw_events from the indexer;
                // wiring it in is Day 2 work. Null, never invented.
                tx_hash: None,
            },
            now,
        );
        let results = checks::run_ladder(vec![rung1]);

        db.write_snapshot(run_id, &chain_name, s).await?;
        db.write_results(run_id, &chain_name, s.agent_id, &results).await?;

        export::write_agent(&export::AgentDocument {
            run_id: run_id.to_string(),
            chain: &chain_name,
            agent_id: s.agent_id,
            token_id: s.token_id.to_string(),
            owner: &s.owner,
            agent_uri: &s.agent_uri,
            block_number: s.block_number,
            checks: &results,
            checker_commit,
            spec_commit: checks::SPEC_COMMIT,
        })?;
    }

    db.close_run(run_id, snapshots.len() as i32, Utc::now()).await?;
    tracing::info!("run {run_id} complete: {} agents", snapshots.len());
    println!("{run_id}");
    Ok(())
}
