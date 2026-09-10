//! The Seller Census's dead man's switch.
//!
//! ```text
//! DATABASE_URL=… SELLER_HEARTBEAT_URL=https://hc-ping.com/… seller-heartbeat
//! ```
//!
//! ## Why this is a second switch and not a second caller of the first
//!
//! The registration census already has `heartbeat`, and the cheap thing would
//! have been to call it at the end of `seller-sweep.sh` too. That would have
//! been actively harmful: it pings when the eleven CHAINS are fresh, so a
//! healthy Thursday seller sweep would have reported the registration census
//! healthy on a week it had not run.
//!
//! That is not hypothetical. On 2026-09-08 the nine-chain job had been failing
//! for two weeks, and the switch was being kept alive by the Base and BSC jobs
//! pinging on their own days. The alarm was fed by a hand that was not the one
//! that had stopped working. Two instruments, two switches, two URLs — hence
//! `SELLER_HEARTBEAT_URL` rather than reusing `HEARTBEAT_URL`, which this
//! binary deliberately ignores even when it is set.
//!
//! ## What it will and will not vouch for
//!
//! It pings only when the newest seller run FINISHED, is younger than the
//! window, and recorded which rungs it attempted. It cannot yet check that the
//! run was published, because the Seller Census has no archive: nothing
//! exports a seller run to the bucket the way `export-run` and
//! `publish-run.sh` do for a chain. The registration heartbeat treats "not in
//! `published-runs.json`" as broken, and this one has no equivalent to read.
//!
//! That gap is stated here rather than papered over, because a switch that
//! implies more than it checked is worse than one that checks less and says
//! so: this vouches that a sweep ran and stored its rows, NOT that a reader
//! can download it. When the archive lands, this binary gets the same
//! published-check the census heartbeat has, and its doc loses this section.

use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;

/// How stale the newest finished seller run may be before the census counts
/// as broken.
///
/// Nine days, and for the same reason the registration census uses nine: the
/// sweep is weekly, so one skipped Thursday for a deliberate reason should not
/// page anybody and two consecutive misses should. Two missed Thursdays cannot
/// both sit inside a nine-day window.
fn max_age_days() -> i64 {
    std::env::var("SELLER_HEARTBEAT_MAX_AGE_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .context("connecting to Postgres")?;

    let max_age = max_age_days();

    // The newest FINISHED run, not the newest run. A failed sweep is stamped
    // with the moment it died, so ordering by `started_at` without filtering
    // on status would let a crash an hour ago vouch for the census — which is
    // how the first seller delta came to report 2,387 sellers "appeared".
    let newest: Option<(uuid::Uuid, chrono::DateTime<chrono::Utc>, Option<Vec<i16>>)> =
        sqlx::query_as(
            "SELECT run_id, finished_at, rungs_attempted FROM seller_runs \
             WHERE status = 'finished' AND finished_at IS NOT NULL \
             ORDER BY finished_at DESC LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .context("finding the newest finished seller run")?;

    let (run_id, finished_at, rungs) = match newest {
        None => anyhow::bail!("no finished seller run exists — heartbeat NOT sent"),
        Some(row) => row,
    };

    let age = (chrono::Utc::now() - finished_at).num_days();
    if age > max_age {
        anyhow::bail!(
            "newest finished seller run ({}) is {age} days old, over the {max_age}d window \
             — heartbeat NOT sent",
            &run_id.to_string()[..8]
        );
    }

    // A run that recorded no `rungs_attempted` cannot say what it asked, and
    // "asked nothing" and "did not write the column" are indistinguishable
    // from outside. Neither is a sweep worth vouching for.
    match rungs {
        None => anyhow::bail!(
            "seller run {} recorded no rungs_attempted — cannot tell what it asked, \
             heartbeat NOT sent",
            &run_id.to_string()[..8]
        ),
        Some(ref r) if r.is_empty() => anyhow::bail!(
            "seller run {} attempted no rungs — heartbeat NOT sent",
            &run_id.to_string()[..8]
        ),
        Some(ref r) => {
            tracing::info!(
                "newest seller run {} is healthy — finished {age}d ago, rungs {:?}",
                &run_id.to_string()[..8],
                r
            );
        }
    }

    // Set-and-empty must behave exactly like unset. `deploy-weekly-sweep.sh`
    // writes `SELLER_HEARTBEAT_URL=` whenever it runs without one, and the
    // registration heartbeat learned this the expensive way: on 2026-08-05 it
    // swept four chains, published every archive, reported every chain
    // healthy, and then failed trying to ping the empty string. A pipeline
    // that does all its work and then reports failure is worse than one that
    // says plainly it had nowhere to report.
    match std::env::var("SELLER_HEARTBEAT_URL")
        .ok()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
    {
        None => {
            tracing::warn!(
                "seller census healthy, but SELLER_HEARTBEAT_URL is unset — nothing was pinged. \
                 Until it is set, a Thursday that stops firing is still invisible."
            );
        }
        Some(url) => {
            let res = reqwest::Client::new()
                .get(&url)
                .timeout(std::time::Duration::from_secs(15))
                .send()
                .await
                .context("pinging the seller heartbeat monitor")?;
            anyhow::ensure!(
                res.status().is_success(),
                "heartbeat monitor answered {}",
                res.status()
            );
            tracing::info!("seller census healthy — heartbeat sent");
        }
    }

    Ok(())
}
