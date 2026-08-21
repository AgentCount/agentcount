//! # seller-delta — what changed since the previous sweep.
//!
//! ```text
//! DATABASE_URL=… seller-delta                 # the latest sweep vs the one before
//! DATABASE_URL=… seller-delta <run-id>        # a specific sweep vs its predecessor
//! DATABASE_URL=… seller-delta --dry-run       # compute and report, write nothing
//! ```
//!
//! METHODOLOGY §10.6. Runs last in a sweep, after the population and every
//! rung the sweep attempted, because a delta computed against a half-written
//! sweep compares the world to a mistake.
//!
//! ## The two things it refuses to do
//!
//! * **It writes no row when there is no predecessor.** A first sweep has
//!   nothing to compare, and "first observation" is not "nothing changed" —
//!   the same absence-is-not-a-status rule the census keeps everywhere. The
//!   API serves a missing delta as a 404, never as zeros.
//! * **It never re-picks the pair.** The predecessor is chosen once, here,
//!   and stored by id. A view that re-picked "the previous sweep" as later
//!   sweeps landed would quietly change a number somebody cited.
//!
//! The arithmetic itself is `sellers::delta::compute`, pure and tested
//! without a database, so a reader can check the rule without running this.

use anyhow::{Context, Result};
use seller_sweeper::store::Db;
use sellers::delta::{self, MethodConfound};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let explicit: Option<Uuid> = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(|a| Uuid::parse_str(a))
        .transpose()
        .context("run id must be a uuid")?;

    let url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = Db::connect(&url).await?;
    let run_id = match explicit {
        Some(id) => id,
        None => db
            .latest_run(sellers::network::BASE)
            .await?
            .context("no seller sweep to compute a delta for")?,
    };

    let after_meta = db.run_meta(run_id).await?;
    let Some(previous) = db.previous_run(run_id, &after_meta.network).await? else {
        // No predecessor. No row, and no zeros pretending to be one.
        tracing::info!(
            "sweep {run_id} is the first on {} — no delta, because \"first observation\" \
             is not \"nothing changed\"",
            after_meta.network
        );
        return Ok(());
    };
    let before_meta = db.run_meta(previous).await?;

    let before = db.rung_statuses(previous).await?;
    let after = db.rung_statuses(run_id).await?;
    let d = delta::compute(&before, &after);

    let confound = MethodConfound::between(
        &before_meta.checker,
        &after_meta.checker,
        &before_meta.catalogs,
        &after_meta.catalogs,
        before_meta.rungs_attempted.as_deref().unwrap_or(&[]),
        after_meta.rungs_attempted.as_deref().unwrap_or(&[]),
    );

    tracing::info!(
        "{} → {}: {} sellers (was {}), +{} appeared, -{} disappeared",
        &previous.to_string()[..8],
        &run_id.to_string()[..8],
        d.sellers_after,
        d.sellers_before,
        d.appeared,
        d.disappeared
    );
    tracing::info!(
        "  reachability: +{} came back, -{} WENT DARK",
        d.came_back,
        d.went_dark
    );
    tracing::info!(
        "  excluded from both by rule: {} refused, {} error, {} unprobed",
        d.excluded_refused,
        d.excluded_error,
        d.excluded_unprobed
    );
    if confound.changed() {
        // The rule §9 states and this instrument inherits: any surface that
        // renders a delta must say when the method moved under it.
        tracing::warn!(
            "  METHOD CHANGED across this pair — checker {}, catalogs {}, rungs {} — \
             an unknown share of the movement above is method, not the world",
            confound.checker_changed,
            confound.catalogs_changed,
            confound.rungs_changed
        );
        if confound.rungs_changed {
            tracing::warn!(
                "    asked before: {:?}; asked after: {:?}",
                before_meta.rungs_attempted,
                after_meta.rungs_attempted
            );
        }
    }

    if dry_run {
        tracing::info!("DRY RUN — nothing was written");
        return Ok(());
    }
    db.write_delta(
        run_id,
        previous,
        &after_meta.network,
        &d,
        &before_meta,
        &after_meta,
        &confound,
    )
    .await?;
    tracing::info!("delta written for {run_id}");
    Ok(())
}
