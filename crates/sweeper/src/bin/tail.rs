//! # tail — keep newly registered agents findable between censuses.
//!
//! ```text
//! DATABASE_URL=… RPC_URL_BASE=… tail                 # poll forever, every TAIL_INTERVAL_SECS
//! DATABASE_URL=… RPC_URL_BASE=… tail --once          # one pass, then exit (cron/Cloud Run job)
//! DATABASE_URL=… RPC_URL_BASE=… tail --once base     # one pass, only this chain
//! ```
//!
//! ## The failure this closes
//!
//! The census pins a block, and that pin is the whole authority of its
//! numbers. It also means an agent minted after the sweep does not exist as
//! far as this site is concerned: search finds nothing, and its permalink
//! 404s. The person most likely to try is the registrant, minutes after
//! minting, and what they conclude is that the site is broken.
//!
//! Discovery is the cheap half. Agent ids are contiguous from 0, so
//! `chain::Registry::highest_agent_id` finds the top of the range in ~17
//! `ownerOf` calls at 60,000 agents, and reading one id is two more calls.
//! What costs hours is everything after discovery — fetching each declared
//! document, probing endpoints, judging seven rungs. This binary does the
//! cheap half continuously and the expensive half never.
//!
//! ## What it will not do
//!
//! It writes exactly two tables, `registration_tail` and
//! `registration_tail_cursor` (migration 0018), plus one column of the first
//! one when a census catches up. Neither has a `run_id` or a foreign key to
//! `runs`, so no rate, finding, delta, archive or published figure can reach
//! these rows: every census aggregate starts at `runs` and joins downward.
//! A tail row carries no rung and no status, because no check was run — and
//! the API refuses to give it a `rungs` array at all, so a client cannot
//! render seven statuses for an agent that has none.
//!
//! ## Which mode to deploy
//!
//! **`--once`, on a scheduler.** The loop mode exists for local runs and for
//! a plain VM, but the deployment this project already has is Cloud Run Jobs
//! plus Cloud Scheduler, and a scheduled one-shot is strictly better there: a
//! crashed tick is visible as a failed job execution rather than as a process
//! that quietly stopped iterating, the interval is changed without a redeploy,
//! and nothing is billed between ticks. `--once` also makes the run-to-
//! completion contract explicit — it exits non-zero if any chain failed, so
//! the scheduler's own retry and alerting apply.
//!
//! ## Bounded work per tick
//!
//! At most [`DEFAULT_MAX_IDS_PER_TICK`] new ids per chain per tick
//! (`TAIL_MAX_IDS_PER_TICK`). A burst mint of 40,000 agents therefore costs a
//! bounded number of RPC calls per tick and is picked up over the following
//! ticks, in id order, instead of turning one poll into an hours-long sweep
//! that no watchdog is watching. The remainder is logged every tick, so a cap
//! that is chronically too small for a chain's mint rate is visible rather
//! than merely slow.

use anyhow::{Context, Result};
use sweeper::store;
use sweeper::tail::{TailPlan, plan_new_ids};

/// How long between polls in loop mode. Five minutes: long enough that a chain
/// sees ~12 head reads an hour from us, short enough that a registrant who
/// mints and then goes looking finds themselves on their first or second try.
const DEFAULT_INTERVAL_SECS: u64 = 300;

fn interval_secs() -> u64 {
    std::env::var("TAIL_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_INTERVAL_SECS)
}

/// The most new ids one chain may be read in one tick.
///
/// 500 ids is 1,000 RPC calls at two reads each — minutes of work at the
/// conservative concurrency this project uses against free-tier providers, and
/// small enough that a tick cannot outlive its own interval by much. The point
/// is not the exact number; it is that the number EXISTS, so no on-chain event
/// can make a poll unbounded.
const DEFAULT_MAX_IDS_PER_TICK: usize = 500;

fn max_ids_per_tick() -> usize {
    std::env::var("TAIL_MAX_IDS_PER_TICK")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_MAX_IDS_PER_TICK)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let once = args.iter().any(|a| a == "--once") || std::env::var("TAIL_ONCE").is_ok();
    let only: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|a| a.trim().to_lowercase())
        .collect();

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = store::Db::connect(&database_url).await?;

    if once {
        let failures = tick(&db, &only).await?;
        anyhow::ensure!(
            failures == 0,
            "{failures} chain(s) failed this pass — see the errors above"
        );
        return Ok(());
    }

    let interval = std::time::Duration::from_secs(interval_secs());
    tracing::info!(
        "registration tail polling every {}s (cap {} new ids per chain per tick)",
        interval.as_secs(),
        max_ids_per_tick()
    );
    loop {
        // A failing tick is logged, never fatal: the tail is a convenience
        // layer over a census that is authoritative without it, and a poller
        // that exits on one bad RPC response stops being the thing that keeps
        // new agents findable.
        match tick(&db, &only).await {
            Ok(0) => {}
            Ok(n) => tracing::warn!("{n} chain(s) failed this tick — continuing"),
            Err(e) => tracing::error!("tick failed entirely: {e:#} — continuing"),
        }
        tokio::time::sleep(interval).await;
    }
}

/// One pass over every enabled chain. Returns how many chains failed.
///
/// A chain's failure is contained: the others still get their tick. Chains are
/// read from the `chains` table, not a list here, for the same reason
/// `heartbeat` does it — a chain enabled in the database and absent from a
/// hardcoded list would silently have no tail.
async fn tick(db: &store::Db, only: &[String]) -> Result<usize> {
    let chains = db.enabled_chains().await?;
    let chains: Vec<String> = if only.is_empty() {
        chains
    } else {
        chains.into_iter().filter(|c| only.contains(c)).collect()
    };
    anyhow::ensure!(!chains.is_empty(), "no enabled chains to poll");

    let mut failures = 0usize;
    for chain in &chains {
        if let Err(e) = poll_chain(db, chain).await {
            tracing::error!("{chain}: tail poll failed: {e:#}");
            failures += 1;
        }
    }
    Ok(failures)
}

/// Discover and record whatever this chain has minted since the tail last
/// looked.
async fn poll_chain(db: &store::Db, chain: &str) -> Result<()> {
    // The backstop half of the supersede. `Db::close_run` already retires the
    // tail rows a finishing run covered; this repeats it for the newest
    // finished run in case that call failed (a database blip, a sweeper build
    // predating migration 0018). It is idempotent — the update only touches
    // rows still marked unswept — and it is why a missed supersede is a
    // temporary display bug rather than a permanent wrong state.
    if let Some((run_id, _)) = db.latest_finished_run(chain).await? {
        match db.supersede_tail(run_id).await {
            Ok(0) => {}
            Ok(n) => tracing::info!("{chain}: retired {n} tail row(s) now covered by the census"),
            Err(e) => tracing::warn!("{chain}: supersede backstop failed: {e:#}"),
        }
    }

    let rpc_var = format!("RPC_URL_{}", chain.to_uppercase());
    let rpc_url = std::env::var(&rpc_var).with_context(|| format!("{rpc_var} must be set"))?;
    let (_chain_id, registry_addr, _reputation, _deploy_block) = db.chain_config(chain).await?;
    // Reconnected each tick rather than held across the loop: a five-minute
    // interval makes the setup cost irrelevant, and a socket that died between
    // ticks (the exact failure `RPC_CALL_TIMEOUT_SECS` exists for) is replaced
    // rather than retried forever.
    let registry = chain::Registry::connect(&rpc_url, &registry_addr).await?;

    // Every read in this tick is pinned to one block, for the same reason a
    // census run is: an id read at one block and its owner at another describe
    // two different moments. The pin here is per tick, not per census — which
    // is precisely why these rows are not census data.
    let block = registry.pinned_block().await?;
    let head = registry.highest_agent_id(block).await?;

    // The baseline is the greater of what the tail has already read and what
    // the last finished census swept. Taking the census into account matters:
    // after a sweep, every id below its high-water mark has real check results,
    // and the tail has no business re-recording them as unchecked.
    let cursor = db.tail_cursor(chain).await?;
    let census_high = db.census_high_water(chain).await?;
    let known = match (cursor.map(|(id, _)| id), census_high) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };

    let TailPlan {
        ids,
        next_cursor,
        remaining,
    } = plan_new_ids(known, head, max_ids_per_tick());

    if ids.is_empty() {
        if let Some(c) = next_cursor {
            db.advance_tail_cursor(chain, c, block).await?;
        }
        tracing::debug!("{chain}: nothing new at block {block} (head {head:?})");
        return Ok(());
    }

    tracing::info!(
        "{chain}: {} new id(s) to read at block {block} ({}..={}){}",
        ids.len(),
        ids[0],
        ids[ids.len() - 1],
        if remaining > 0 {
            format!(", {remaining} more deferred to the next tick by the per-tick cap")
        } else {
            String::new()
        }
    );

    // Sequential and in ascending id order, on purpose. The cursor may only
    // advance past ids that are actually stored, so a failure in the middle
    // must leave a CONTIGUOUS prefix behind it — concurrency here would
    // scatter the successes and make "where do we resume" unanswerable
    // without re-reading everything.
    let mut inserted = 0usize;
    let mut last_ok: Option<u64> = None;
    for id in ids {
        match registry.snapshot(id, block).await {
            Ok(s) => {
                match db.record_tail(chain, &s).await {
                    Ok(true) => inserted += 1,
                    // Already there: another poller, or this one before a
                    // restart. Idempotence, working — not a failure, and the
                    // cursor may still advance past it.
                    Ok(false) => {}
                    Err(e) => {
                        tracing::warn!(
                            "{chain}: could not store agent {id}: {e:#} — stopping this \
                             tick here; the next one resumes at {id}"
                        );
                        break;
                    }
                }
                last_ok = Some(id);
            }
            Err(e) => {
                tracing::warn!(
                    "{chain}: could not read agent {id} at block {block}: {e:#} — stopping \
                     this tick here; the next one resumes at {id}"
                );
                break;
            }
        }
    }

    if let Some(c) = last_ok {
        db.advance_tail_cursor(chain, c, block).await?;
        tracing::info!(
            "{chain}: {inserted} new agent(s) recorded in the tail, cursor now at {c} \
             (block {block})"
        );
    } else {
        // Not one id was read. Leave the cursor exactly where it was: moving
        // it would silently skip agents nobody would ever discover again.
        tracing::warn!("{chain}: no id could be read this tick — cursor left unchanged");
    }
    Ok(())
}
