//! # findings — the homepage's five numbers, computed once and stored.
//!
//! ```text
//! DATABASE_URL=… findings bsc            # the newest finished run on a chain
//! DATABASE_URL=… findings <run-id>       # one specific run
//! DATABASE_URL=… findings --all          # every run that has results
//! ```
//!
//! Run after a sweep finishes, and after `liveness` — not because any finding
//! reads rung 6 today, but because rung 6 is written into the same run and the
//! stored figures should describe the run as it will be published, not as it
//! was halfway through.
//!
//! ## Why this binary exists
//!
//! `GET /api/runs/{id}/findings` used to count on every request. Two of the
//! five findings cannot be answered from an index — one reads `evidence`, the
//! other groups by `(chain, agent_id)` while filtering on `rung` — so neither
//! can be an index-only scan. Measured on production for the 2026-08 BNB Chain
//! run (251,782 agents), cache warm: 5.0 s for the first and 9.2 s for the
//! second, about 2.5 GB of reads between them. The API caps a request at ten
//! seconds and returns 408, so the endpoint could not answer for that run at
//! all — which took the homepage's all-chains figure down with it, because the
//! aggregate sums one findings document per chain.
//!
//! An earlier version of this comment said "roughly 550 seconds", a figure that
//! was reconstructed on a workstation rather than measured here. Migration 0021
//! records the correction and the plans behind these numbers.
//!
//! These numbers stop changing the moment a sweep closes. Computing them once
//! is not a cache; it is the same decision `delta` already made for the same
//! reason (migration 0016), and the recount stays available: every figure is an
//! aggregate over `check_results`, and re-running this binary reproduces it.
//!
//! ## What it does NOT do
//!
//! It does not define a finding. The arithmetic is `ls_run_findings()` in
//! migration 0021, which the API also calls directly for a run that has no
//! stored row — one implementation, three callers. A second copy here would be
//! a second answer waiting to happen, and it would be the copy nobody
//! remembers to update.
//!
//! It writes nothing to `check_results` and reads no chain and no network.

use anyhow::{Context, Result};
use sweeper::store;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let arg = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "base".to_string());
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = store::Db::connect(&database_url).await?;

    // Three ways to name the work, resolved to a list of runs before any of it
    // starts, so the log says up front what is going to be recomputed.
    let runs: Vec<Uuid> = if arg == "--all" {
        db.runs_with_results()
            .await?
            .into_iter()
            .map(|(run_id, _chain)| run_id)
            .collect()
    } else if let Ok(run_id) = Uuid::parse_str(&arg) {
        vec![run_id]
    } else {
        match db.latest_finished_run(&arg).await? {
            Some((run_id, _)) => vec![run_id],
            None => {
                // Not an error. A chain with no finished run has nothing to
                // summarise, and the weekly job must not fail a whole sweep
                // over a chain that was skipped this week.
                tracing::info!("chain {arg} has no finished run — nothing to compute");
                return Ok(());
            }
        }
    };

    for run_id in runs {
        // Every large run costs roughly what one old `/findings` request cost.
        // Said out loud per run, because on BNB Chain that is minutes and a
        // silent process looks hung.
        tracing::info!("computing findings for run {run_id} (a full pass over its check results)");
        let written = db.write_findings(run_id).await?;
        if written == 0 {
            tracing::warn!("run {run_id} has no check results — no findings written");
            continue;
        }
        for (key, numerator, denominator) in db.run_findings(run_id).await? {
            tracing::info!("  {key}: {numerator} / {denominator}");
        }
    }

    Ok(())
}
