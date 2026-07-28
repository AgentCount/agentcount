//! # sweeper — run the conformance ladder over one chain, once.
//!
//! A run is the unit of work and the unit of citation: it pins a block, reads
//! every agent's current state, answers the rungs it can, and writes both the
//! database rows and the `data/<run_id>/` export. Runs are immutable; to get
//! newer answers you take a new run, never edit an old one. Resuming (see
//! [`sweep_resume`]) does not break that: it adds rows to a run that never
//! finished, it never edits a row already written.
//!
//! Day 1 answers rung 1 only. Rungs 2-7 are ABSENT from the output rather than
//! reported as `skipped` — "we did not ask" and "we could not ask" are
//! different claims and the schema keeps them different.

mod export;
mod store;

use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::Utc;
use futures::stream::{self, StreamExt};
use uuid::Uuid;

/// How many `ownerOf`/`tokenURI` pairs to read at once. Conservative: a public
/// RPC endpoint is a shared resource and this is not a race. Lowered from 8
/// after Task 8's first live sweep hit Alchemy's free-tier "compute units per
/// second" cap immediately — override with `RPC_CONCURRENCY` without
/// recompiling.
const DEFAULT_RPC_CONCURRENCY: usize = 3;

fn rpc_concurrency() -> usize {
    std::env::var("RPC_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_RPC_CONCURRENCY)
}

/// Sweep only the first N discovered agent ids, if set. Exists so a bounded
/// pilot run can validate the whole pipeline (DB rows, exports, rerun
/// command) before committing to a multi-hour full sweep of the real
/// population. When set, it MUST show up in the run's `rerun_command` —
/// a run that swept 2,000 of 59,998 agents but whose rerun command implies a
/// full sweep would misrepresent what was actually measured.
fn sweep_max_agents() -> Option<usize> {
    std::env::var("SWEEP_MAX_AGENTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
}

/// Resume an existing run instead of opening a new one. Set to a `run_id` a
/// previous sweep printed. Exists because a ~60,000-agent sweep runs for
/// hours, and a crash partway through — an RPC failure, a value the database
/// refuses (the NUL-byte hazard [`store::escape_nuls_for_postgres`-adjacent
/// code] guards against), a wedged connection, Ctrl-C — should not force
/// starting over from agent 0.
fn sweep_resume() -> Result<Option<Uuid>> {
    match std::env::var("SWEEP_RESUME") {
        Ok(s) => Ok(Some(Uuid::parse_str(&s).with_context(|| {
            format!("SWEEP_RESUME={s} is not a valid run id")
        })?)),
        Err(_) => Ok(None),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let chain_arg = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "base".to_string());
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = store::Db::connect(&database_url).await?;

    // Resuming reloads chain, pinned_block, and every provenance column from
    // the EXISTING run, rather than deriving them fresh — see `sweep_resume`.
    let resume_run_id = sweep_resume()?;
    let resumed = match resume_run_id {
        Some(run_id) => {
            let r = db.load_run(run_id).await?;
            if r.chain != chain_arg {
                tracing::warn!(
                    "SWEEP_RESUME={run_id} was recorded for chain {}; ignoring the \
                     command-line chain argument {chain_arg:?}",
                    r.chain
                );
            }
            Some((run_id, r))
        }
        None => None,
    };
    let chain_name = resumed
        .as_ref()
        .map(|(_, r)| r.chain.clone())
        .unwrap_or(chain_arg);

    let rpc_var = format!("RPC_URL_{}", chain_name.to_uppercase());
    let rpc_url = std::env::var(&rpc_var).with_context(|| format!("{rpc_var} must be set"))?;
    let (chain_id, registry_addr, deploy_block) = db.chain_config(&chain_name).await?;
    let registry = chain::Registry::connect(&rpc_url, &registry_addr).await?;

    // `deploy_block` is no longer used for enumeration (agent ids are found
    // by binary search on `ownerOf` existence, not by scanning logs from
    // deploy to head — see crates/chain/src/registry.rs), but the column
    // still describes the chain and stays wired for chain_config's other
    // callers.
    let _ = deploy_block;

    let (
        run_id,
        pinned,
        schema_version,
        checker_version,
        checker_commit,
        spec_commit,
        rerun,
        started_at,
        already_swept,
    ) = match resumed {
        Some((run_id, r)) => {
            let already_swept = db.swept_agent_ids(run_id, &chain_name).await?;
            tracing::info!(
                "resuming run {run_id} on {chain_name} at pinned block {} — \
                     {} agent(s) already swept, resuming the remainder",
                r.pinned_block,
                already_swept.len()
            );
            (
                run_id,
                r.pinned_block,
                r.schema_version,
                r.checker_version,
                r.checker_commit,
                r.spec_commit,
                r.rerun_command,
                r.started_at.to_rfc3339(),
                already_swept,
            )
        }
        None => {
            let pinned = registry.pinned_block().await?;
            tracing::info!("sweeping {chain_name} at block {pinned}");

            let run_id = Uuid::new_v4();
            let checker_commit = env!("CHECKER_COMMIT").to_string();
            let max_agents = sweep_max_agents();
            // The rerun command must describe what THIS run actually
            // swept. A pilot capped by SWEEP_MAX_AGENTS is not reproduced
            // by the bare command below — omitting the cap here would
            // make the archived run claim a full sweep it never did.
            let rerun = match max_agents {
                Some(n) => format!(
                    "SWEEP_MAX_AGENTS={n} cargo run -p sweeper -- {chain_name}   # at block {pinned}"
                ),
                None => format!("cargo run -p sweeper -- {chain_name}   # at block {pinned}"),
            };

            db.open_run(&store::RunMeta {
                run_id,
                chain: chain_name.clone(),
                pinned_block: pinned,
                schema_version: checks::SCHEMA_VERSION,
                checker_version: checks::CHECKER_VERSION.to_string(),
                checker_commit: checker_commit.clone(),
                spec_commit: checks::SPEC_COMMIT.to_string(),
                rerun_command: rerun.clone(),
            })
            .await?;

            (
                run_id,
                pinned,
                checks::SCHEMA_VERSION,
                checks::CHECKER_VERSION.to_string(),
                checker_commit,
                checks::SPEC_COMMIT.to_string(),
                rerun,
                Utc::now().to_rfc3339(),
                HashSet::new(),
            )
        }
    };
    let checker_commit = checker_commit.as_str();
    let checker_version = checker_version.as_str();
    let spec_commit = spec_commit.as_str();

    let max_agents = sweep_max_agents();
    // Enumerated at the PINNED block (the original one, if resuming) so the
    // population matches what the first session saw, not whatever exists on
    // chain right now.
    let mut ids = registry.enumerate_agent_ids(pinned).await?;
    let discovered = ids.len();
    if let Some(n) = max_agents {
        ids.truncate(n);
    }
    // `planned` is this run's TOTAL intended scope — cumulative across every
    // session that has worked on it, not just this one. It equals
    // `already_swept.len() + ids.len()` below by construction (the same list
    // just gets filtered), which is what keeps the swept/unreadable math at
    // the end honest without having to remember a prior session's counts.
    let planned = ids.len();
    ids.retain(|id| !already_swept.contains(id));
    let remaining = ids.len();
    tracing::info!(
        "{discovered} agent ids discovered; {planned} in scope for this run \
         ({} already swept, {remaining} remaining this session){}",
        already_swept.len(),
        max_agents
            .map(|n| format!(" (SWEEP_MAX_AGENTS={n})"))
            .unwrap_or_default()
    );

    // Read current state for each id, bounded. `buffer_unordered` keeps at most
    // RPC_CONCURRENCY reads in flight; results arrive out of order, which is
    // fine because each carries its own agent_id.
    // The manifest is written BEFORE the sweep, so a run that dies partway
    // still leaves a readable, self-describing directory on disk — and then
    // REWRITTEN at the end with what actually happened. Writing it only once,
    // up front, would mean the artefact a reader downloads reports the
    // population we intended to sweep while the files beside it hold however
    // many we managed: the incompleteness would be discoverable only by
    // counting rows, which is exactly what this project promises never to
    // make someone do.
    let manifest = |swept: Option<usize>, unreadable: Option<usize>, finished: Option<String>| {
        export::RunManifest {
            run_id: run_id.to_string(),
            chain: &chain_name,
            chain_id: chain_id as u64,
            registry: &registry_addr,
            pinned_block: pinned,
            started_at: started_at.clone(),
            schema_version,
            checker_version,
            checker_commit,
            spec_commit,
            rerun_command: &rerun,
            agent_count: planned,
            swept,
            unreadable,
            finished_at: finished,
        }
    };
    export::write_manifest(&manifest(None, None, None))?;

    // Persist each agent AS IT ARRIVES rather than collecting the whole
    // population first. At 60,000 agents a sweep runs for hours, and a
    // collect-then-write shape means a crash, a dropped connection, or a
    // Ctrl-C at hour three discards every read — plus the database shows
    // nothing until the very end, so there is no way to tell a working sweep
    // from a wedged one.
    let mut stream = stream::iter(ids)
        .map(|id| {
            let registry = &registry;
            async move { (id, registry.snapshot(id, pinned).await) }
        })
        .buffer_unordered(rpc_concurrency());

    // Session-local, but see the `planned` comment above: because `ids` here
    // is exactly `planned` minus `already_swept`, every id in it is attempted
    // exactly once (success or failure), so `already_swept.len() + swept +
    // unreadable == planned` holds whether this is a fresh run (already_swept
    // empty) or a resumed one — no need to have persisted a prior session's
    // failure count anywhere to report the true cumulative totals below.
    let mut swept = 0usize;
    let mut unreadable = 0usize;

    while let Some((id, result)) = stream.next().await {
        let s = match result {
            Ok(s) => s,
            Err(e) => {
                // An RPC failure is OUR problem, not the agent's: leave the
                // agent out of this run rather than recording a `fail` about
                // them. The count is reported at the end so the omission is
                // visible instead of silent.
                tracing::warn!("snapshot({id}) failed: {e:#}");
                unreadable += 1;
                continue;
            }
        };
        let s = &s;
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
        db.write_results(run_id, &chain_name, s.agent_id, &results)
            .await?;

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
            spec_commit,
        })?;

        swept += 1;
        if swept % 500 == 0 {
            tracing::info!(
                "{swept}/{remaining} agents swept this session ({unreadable} unreadable this session)"
            );
        }
    }

    let finished = Utc::now();
    // Cumulative across every session this run has had, per the invariant
    // documented above the loop.
    let total_swept = already_swept.len() + swept;
    db.close_run(run_id, total_swept as i32, finished).await?;
    // Rewrite the manifest so the downloadable artefact matches the rows.
    export::write_manifest(&manifest(
        Some(total_swept),
        Some(unreadable),
        Some(finished.to_rfc3339()),
    ))?;
    if unreadable > 0 {
        // Say it loudly: a census missing agents is not a complete census, and
        // the gap must never be discovered later from a row count.
        tracing::warn!(
            "run {run_id}: {unreadable} of {planned} agents could not be read \
             and are ABSENT from this run — not recorded as failures"
        );
    }
    tracing::info!("run {run_id} complete: {total_swept} of {planned} agents");
    println!("{run_id}");
    Ok(())
}
