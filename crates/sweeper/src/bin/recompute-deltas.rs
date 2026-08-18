//! # recompute-deltas — refresh every stored delta through today's arithmetic.
//!
//! ```text
//! DATABASE_URL=… recompute-deltas            # every delta, DRY RUN
//! DATABASE_URL=… recompute-deltas --apply    # every delta, for real
//! ```
//!
//! ## Why a stored row is rewritten at all
//!
//! A delta is derived, not measured — `run_deltas` is a cache of what
//! `sweeper::delta::compute` said about two archived runs, kept so the API
//! serves a stored comparison rather than computing one per request. When the
//! RULE changes (as on 2026-08-18, when rung-2 `error` transitions joined
//! `refused` outside the headline series), the archived measurements are
//! untouched and still right; the stored derivation is stale and would keep
//! publishing churn the methodology no longer claims. Refreshing it is the
//! same act as the delta being written at sweep time, done again.
//!
//! No network request is made and no chain is read. The measurements this
//! derives from are never touched — that is `backfill-refused`'s job, and
//! only for the narrow renaming it documents.
//!
//! Dry run by default, because this writes to published data and a binary
//! that does that on a typo is a defect of its own. The dry run prints
//! exactly the same old → new table `--apply` would leave behind.

use anyhow::{Context, Result};
use sweeper::store;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let apply = std::env::args().skip(1).any(|a| a == "--apply");
    if !apply {
        tracing::warn!("DRY RUN — nothing will be written. Re-run with --apply to rewrite.");
    }

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = store::Db::connect(&database_url).await?;

    let recomputed = sweeper::recompute::all(&db, apply).await?;
    if recomputed.is_empty() {
        tracing::warn!("no stored deltas — nothing to do");
        return Ok(());
    }

    // ── The report. This is the deliverable, not a progress log ──────────
    println!(
        "\ndeltas — stored → recomputed, per run{}",
        if apply {
            ""
        } else {
            "  (PROJECTED — dry run)"
        }
    );
    println!(
        "{:<38} {:<8} {:>22} {:>22}",
        "run", "chain", "newly_resolving", "stopped_resolving"
    );
    for r in &recomputed {
        println!(
            "{:<38} {:<8} {:>22} {:>22}",
            r.run_id,
            r.chain,
            fmt(r.stored_newly_resolving, r.counts.newly_resolving),
            fmt(r.stored_stopped_resolving, r.counts.stopped_resolving),
        );
    }

    if !apply {
        println!("\nDRY RUN — nothing was written. Re-run with --apply.");
    }
    Ok(())
}

fn fmt(stored: i32, recomputed: i64) -> String {
    if i64::from(stored) == recomputed {
        format!("{stored}")
    } else {
        format!("{stored}→{recomputed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unchanged_series_prints_one_number_and_a_changed_one_prints_both() {
        // The report is the deliverable: a reader must be able to see at a
        // glance which rows the rule change moved.
        assert_eq!(fmt(12, 12), "12");
        assert_eq!(fmt(4479, 2), "4479→2");
    }
}
