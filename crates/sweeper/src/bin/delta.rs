//! # delta — what changed between two runs on one chain.
//!
//! ```text
//! DATABASE_URL=… delta base                      # newest two finished runs
//! DATABASE_URL=… delta base <new-run> <old-run>  # a specific pair
//! ```
//!
//! Run after a sweep finishes. Compares it against the previous finished run
//! on the same chain and writes one `run_deltas` row.
//!
//! ## The number this exists for
//!
//! Registration counts go up, and everyone publishes them. **Agents that
//! stopped resolving** is the one nobody else can produce, because it requires
//! having asked the same question of the same population at two pinned blocks
//! and kept both answers. A registry that only ever counts arrivals cannot
//! tell you whether anything is still there.
//!
//! ## What is deliberately not counted as a change
//!
//! Three things, all of them in `sweeper::delta` with the reasoning attached:
//! an agent that is only in one of the two runs, a rung that has a row on only
//! one side, and — since 2026-08-06 — any transition into or out of `refused`.
//! That last one is the rule that keeps a rate limit of our own making from
//! being published as 19,983 agents going dark; see that module's doc.
//!
//! This binary is the database half only. Every number it writes comes from
//! [`sweeper::delta::compute`], which `backfill-refused` also calls, so a
//! recomputed delta and a freshly computed one cannot disagree.

use anyhow::{Context, Result};
use sweeper::{delta, store};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let chain = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "base".to_string());
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = store::Db::connect(&database_url).await?;

    let (new_run, old_run) = match (std::env::args().nth(2), std::env::args().nth(3)) {
        (Some(a), Some(b)) => (Uuid::parse_str(&a)?, Uuid::parse_str(&b)?),
        _ => {
            let runs = db.finished_runs(&chain, 2).await?;
            if runs.len() < 2 {
                // Not an error. A chain's first sweep has nothing to compare
                // against, and writing a row of zeroes would read as "nothing
                // changed" rather than "this is the first observation".
                tracing::info!(
                    "chain {chain} has fewer than two finished runs — no delta to compute"
                );
                return Ok(());
            }
            (runs[0], runs[1])
        }
    };
    tracing::info!("delta for {chain}: {new_run} against {old_run}");

    let after = db.rung_statuses(new_run).await?;
    let before = db.rung_statuses(old_run).await?;
    let counts = delta::compute(&before, &after);

    // The confound. A delta is only a statement about the world if both runs
    // asked the same questions, and when the checker changed between them some
    // agents moved because WE did. Recorded rather than assumed away — see
    // migration 0016 for the case that made this non-optional.
    let (checker_after, schema_after) = db.run_provenance(new_run).await?;
    let (checker_before, schema_before) = db.run_provenance(old_run).await?;
    if checker_after != checker_before || schema_after != schema_before {
        tracing::warn!(
            "these runs were judged by DIFFERENT checker builds \
             ({checker_before}/schema {schema_before} → {checker_after}/schema {schema_after}); \
             an unknown share of the flips below is method, not the world. \
             Any published figure must say so."
        );
    }

    db.write_delta(&store::DeltaWrite {
        run_id: new_run,
        previous_run_id: old_run,
        chain: &chain,
        agents_before: counts.agents_before as i32,
        agents_after: counts.agents_after as i32,
        newly_registered: counts.newly_registered as i32,
        disappeared: counts.disappeared as i32,
        newly_resolving: counts.newly_resolving as i32,
        stopped_resolving: counts.stopped_resolving as i32,
        flips: &counts.flips_json(),
        checker_before: &checker_before,
        checker_after: &checker_after,
        schema_before,
        schema_after,
    })
    .await?;

    // The excluded transitions are logged rather than dropped in silence: a
    // sweep where they are large is a sweep whose politeness settings want
    // looking at, and that is invisible if the only thing printed is the
    // (correctly) small churn number.
    let declined: i64 = counts
        .flips
        .iter()
        .filter(|f| f.rung == 2 && (f.to == delta::NOT_CHURN || f.from == delta::NOT_CHURN))
        .map(|f| f.agents)
        .sum();
    tracing::info!(
        "delta written: {} agents (was {}), +{} registered, -{} disappeared, \
         +{} resolving, -{} STOPPED resolving \
         ({declined} rung-2 transitions in or out of `{}` excluded from both)",
        counts.agents_after,
        counts.agents_before,
        counts.newly_registered,
        counts.disappeared,
        counts.newly_resolving,
        counts.stopped_resolving,
        delta::NOT_CHURN,
    );
    Ok(())
}
