//! Census tool for `analysis/attestation-rate.md`'s §3 follow-up: enumerate
//! `getClients(agentId)` for EVERY rung-7 `attested` agent in a pinned run,
//! at the exact block that run judged — replacing a 300-agent sample with a
//! full count. Analysis scratch only:
//!
//! - No `crates/*` production code is touched. `chain::Reputation::feedback`
//!   is called exactly as every sweeper run already calls it (same
//!   `getClients` → `getSummary` shape, same `retry_throttled` backoff) —
//!   see `crates/chain/src/reputation.rs`. This binary only decides WHICH
//!   agent ids to call it for and WHERE to persist the result.
//! - No production table (`check_results`, `agent_snapshots`, `http_archive`,
//!   `runs`) is written to, only read from. Results land in two new,
//!   clearly-named scratch tables — see `ensure_schema` below — that nothing
//!   else in the codebase reads.
//!
//! Resumable by construction: every agent already recorded in
//! `analysis_attested_clients_progress` for this run (success OR exhausted
//! failure) is skipped on the next invocation, so killing this process and
//! re-running the same command picks up where it left off rather than
//! re-spending an hour of throttled RPC calls.
//!
//! ```sh
//! DATABASE_URL=... RPC_URL_BASE=... cargo run --release
//! ```

use std::collections::HashSet;
use std::time::Instant;

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// The run this census describes — pinned in `analysis/attestation-rate.md`.
/// The one number in this program that must never silently drift: every
/// downstream count is meaningless if it stops describing the block the run
/// itself judged. Overridable for reuse against a different run, but always
/// logged so a reader of the resulting table can tell which run produced it.
const DEFAULT_RUN_ID: &str = "cfbfcc01-fdaf-409f-9bed-abf706d865c7";
const DEFAULT_BLOCK: u64 = 49_262_617;
const DEFAULT_REPUTATION_REGISTRY: &str = "0x8004baa17c55a88189ae136b182e5fda19de9b63";
/// Matches `crates/sweeper`'s own default and the task brief's instruction
/// to keep this low — same free-tier RPC endpoint, same throttle.
const DEFAULT_CONCURRENCY: usize = 3;

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_or_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn env_or_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Create the two scratch tables if they don't already exist. Named with an
/// `analysis_` prefix precisely so nobody mistakes them for part of the
/// product schema (`migrations/` never mentions them) — they exist to make
/// this census re-derivable without re-reading the chain, nothing else reads
/// them.
async fn ensure_schema(pool: &PgPool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS analysis_attested_clients (
            run_id         UUID    NOT NULL,
            agent_id       BIGINT  NOT NULL,
            client_address TEXT    NOT NULL,
            PRIMARY KEY (run_id, agent_id, client_address)
        )",
    )
    .execute(pool)
    .await
    .context("creating analysis_attested_clients")?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS analysis_attested_clients_progress (
            run_id       UUID        NOT NULL,
            agent_id     BIGINT      NOT NULL,
            status       TEXT        NOT NULL CHECK (status IN ('ok', 'failed')),
            client_count INT,
            error        TEXT,
            fetched_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
            PRIMARY KEY (run_id, agent_id)
        )",
    )
    .execute(pool)
    .await
    .context("creating analysis_attested_clients_progress")?;

    Ok(())
}

async fn attested_agent_ids(pool: &PgPool, run_id: Uuid) -> Result<Vec<u64>> {
    let rows = sqlx::query(
        "SELECT agent_id FROM check_results \
         WHERE run_id = $1 AND rung = 7 AND status = 'pass' \
         ORDER BY agent_id",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .context("loading rung-7 attested agent ids")?;
    Ok(rows
        .into_iter()
        .map(|r| r.get::<i64, _>("agent_id") as u64)
        .collect())
}

async fn already_processed(pool: &PgPool, run_id: Uuid) -> Result<HashSet<u64>> {
    let rows = sqlx::query(
        "SELECT agent_id FROM analysis_attested_clients_progress WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .context("loading already-processed agent ids")?;
    Ok(rows
        .into_iter()
        .map(|r| r.get::<i64, _>("agent_id") as u64)
        .collect())
}

/// Persist one agent's outcome (client list or failure) in a single
/// transaction so `analysis_attested_clients_progress` and
/// `analysis_attested_clients` can never disagree about whether an agent
/// was recorded.
async fn persist_ok(pool: &PgPool, run_id: Uuid, agent_id: u64, clients: &[String]) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO analysis_attested_clients_progress (run_id, agent_id, status, client_count) \
         VALUES ($1, $2, 'ok', $3) \
         ON CONFLICT (run_id, agent_id) DO NOTHING",
    )
    .bind(run_id)
    .bind(agent_id as i64)
    .bind(clients.len() as i32)
    .execute(&mut *tx)
    .await
    .context("recording progress (ok)")?;

    for client in clients {
        sqlx::query(
            "INSERT INTO analysis_attested_clients (run_id, agent_id, client_address) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (run_id, agent_id, client_address) DO NOTHING",
        )
        .bind(run_id)
        .bind(agent_id as i64)
        .bind(client)
        .execute(&mut *tx)
        .await
        .context("recording a client address")?;
    }

    tx.commit().await?;
    Ok(())
}

async fn persist_failed(pool: &PgPool, run_id: Uuid, agent_id: u64, error: &str) -> Result<()> {
    // Truncate defensively — some RPC error bodies embed the entire reverted
    // calldata; there is no need to store megabytes of hex per failure.
    let truncated: String = error.chars().take(2000).collect();
    sqlx::query(
        "INSERT INTO analysis_attested_clients_progress (run_id, agent_id, status, error) \
         VALUES ($1, $2, 'failed', $3) \
         ON CONFLICT (run_id, agent_id) DO NOTHING",
    )
    .bind(run_id)
    .bind(agent_id as i64)
    .bind(truncated)
    .execute(pool)
    .await
    .context("recording progress (failed)")?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let rpc_url = std::env::var("RPC_URL_BASE").context("RPC_URL_BASE must be set")?;
    let run_id_str = env_or("RUN_ID", DEFAULT_RUN_ID);
    let run_id = Uuid::parse_str(&run_id_str).context("RUN_ID must be a UUID")?;
    let block = env_or_u64("PINNED_BLOCK", DEFAULT_BLOCK);
    let reputation_addr = env_or("REPUTATION_REGISTRY", DEFAULT_REPUTATION_REGISTRY);
    let concurrency = env_or_usize("CONCURRENCY", DEFAULT_CONCURRENCY);

    tracing::info!(
        "census: run {run_id}, block {block}, reputation registry {reputation_addr}, \
         concurrency {concurrency}"
    );

    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .context("connecting to Postgres")?;
    ensure_schema(&pool).await?;

    let all_ids = attested_agent_ids(&pool, run_id).await?;
    let total = all_ids.len();
    let done = already_processed(&pool, run_id).await?;
    let remaining: Vec<u64> = all_ids.into_iter().filter(|id| !done.contains(id)).collect();

    tracing::info!(
        "{total} attested agents total; {} already recorded from a prior session; \
         {} remaining this session",
        done.len(),
        remaining.len()
    );

    if remaining.is_empty() {
        tracing::info!("nothing left to do — census already complete for this run");
        return Ok(());
    }

    let rep = chain::Reputation::connect(&rpc_url, &reputation_addr).await?;
    let rep = &rep;

    let start = Instant::now();
    let mut stream = stream::iter(remaining.into_iter())
        .map(|id| async move { (id, rep.feedback(id, block).await) })
        .buffer_unordered(concurrency);

    let mut ok_count = 0usize;
    let mut failed_count = 0usize;
    let mut processed = 0usize;

    while let Some((agent_id, result)) = stream.next().await {
        match result {
            Ok(reads) => {
                persist_ok(&pool, run_id, agent_id, &reads.clients).await?;
                ok_count += 1;
            }
            Err(e) => {
                let msg = format!("{e:#}");
                tracing::warn!("agent {agent_id}: unreadable — {msg}");
                persist_failed(&pool, run_id, agent_id, &msg).await?;
                failed_count += 1;
            }
        }
        processed += 1;
        if processed % 250 == 0 {
            let elapsed = start.elapsed().as_secs_f64();
            let rate = processed as f64 / elapsed.max(0.001);
            tracing::info!(
                "{processed} processed this session ({ok_count} ok, {failed_count} failed) \
                 — {elapsed:.0}s elapsed, {rate:.2} agents/s"
            );
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    tracing::info!(
        "session complete: {processed} processed ({ok_count} ok, {failed_count} failed) \
         in {elapsed:.0}s"
    );

    let final_done = already_processed(&pool, run_id).await?;
    let final_failed: i64 =
        sqlx::query("SELECT count(*) AS n FROM analysis_attested_clients_progress \
                      WHERE run_id = $1 AND status = 'failed'")
            .bind(run_id)
            .fetch_one(&pool)
            .await?
            .get("n");
    tracing::info!(
        "cumulative: {}/{total} attested agents recorded ({} failed, {} ok)",
        final_done.len(),
        final_failed,
        final_done.len() as i64 - final_failed
    );

    Ok(())
}
