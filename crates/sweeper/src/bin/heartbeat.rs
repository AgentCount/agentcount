//! # heartbeat — notice when the census stops happening.
//!
//! ```text
//! DATABASE_URL=… HEARTBEAT_URL=https://… heartbeat        # check, then ping
//! DATABASE_URL=… heartbeat                               # check only, no ping
//! ```
//!
//! ## The failure this exists for
//!
//! Everything else that can go wrong with a sweep produces something. A crash
//! exits non-zero and the job reports it. A hang trips the sweeper's own stall
//! watchdog, which exits 75. A database that refuses writes fails loudly.
//!
//! **A schedule that stops firing produces nothing at all.** No log line, no
//! exit code, no alert — because nothing ran. The Cloud Scheduler job is
//! disabled by a permissions change, or the job's image reference goes stale,
//! or someone pauses it during an incident and forgets, and the census simply
//! stops. Everything looks healthy, because health is measured by things that
//! run, and the thing that would report the problem is the thing that is not
//! running.
//!
//! The only signal for that failure is an ABSENCE — no finished run for a
//! chain in longer than the schedule's period. Absence cannot be detected from
//! inside the job. It needs something that runs on its own clock and expects
//! to hear from the census.
//!
//! ## Two separable jobs, and only one of them is on by default
//!
//! Both start from the same question, asked of the database and answered
//! definitely: does every enabled chain have a finished run inside the
//! freshness window, and did that run's data actually reach the public bucket?
//!
//! **1. The self-check, always on.** If the answer is no, this exits non-zero
//! and fails the weekly job, which the platform's own failure alerting
//! catches. No external service, nothing to configure.
//!
//! This is the part that earns its place, and the case it earns it on is
//! PARTIAL publication: three chains upload, one does not, the job exits 0 and
//! the week looks fine. Total absence is easy for a person to notice; a
//! missing quarter of the data inside a report that arrived on time is not.
//!
//! **2. The dead man's ping, off unless `HEARTBEAT_URL` is set.** Pings an
//! outside monitor when everything is healthy; miss two and the monitor
//! alerts. That catches the scheduler never firing at all — but at a weekly
//! cadence, with a person reading and publishing each result, that person
//! already is the dead man's switch. A report that does not arrive on Monday
//! is noticed about as fast as a monitor would notice it, so an external
//! service to detect it is a moving part earning nothing. Worth setting if
//! the cadence gets faster, if results stop being read by a human every time,
//! or if nobody would notice a quiet month.
//!
//! The monitor, when used, is deliberately not ours: a watchdog hosted on the
//! same infrastructure as the thing it watches fails in the same outage, and
//! would be exactly as silent as the schedule it was meant to catch.
//!
//! ## Where it runs in the weekly job, and why that ORDER
//!
//! Last. After the sweep, after rung 6, after the delta, after the export is
//! archived and uploaded, after the checksum is written, after the run summary
//! is committed. The ping is not "the process finished" — it is **"this
//! week's data is published and verifiable"**, and it must be impossible for
//! it to fire while any of that is untrue.
//!
//! That is why this re-reads the published index from disk rather than
//! trusting the job's own exit codes. A step that reported success but wrote
//! nothing is exactly the kind of failure a heartbeat exists to catch, and a
//! heartbeat that trusts the pipeline it monitors is a heartbeat that agrees
//! with it about everything, including being wrong.

use std::collections::HashSet;

use anyhow::{Context, Result};
use sweeper::store;

/// How stale a chain's newest published run may be before the census counts
/// as broken.
///
/// Nine days, for a weekly schedule. Not seven: a run that starts Monday and
/// finishes Tuesday, plus one skipped week for a deliberate reason, should not
/// page anybody. Two consecutive misses should — and at nine days, two missed
/// Mondays cannot both stay inside the window.
fn max_age_days() -> i64 {
    std::env::var("HEARTBEAT_MAX_AGE_DAYS")
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
    let db = store::Db::connect(&database_url).await?;
    let max_age = max_age_days();

    // Which chains are supposed to be swept, from the database rather than a
    // list here. A chain enabled in `chains` and never swept is exactly the
    // silence this is for, and a hardcoded list would omit it by construction.
    let chains: Vec<String> =
        sqlx::query_scalar("SELECT chain FROM chains WHERE enabled ORDER BY chain")
            .fetch_all(&db.pool)
            .await
            .context("listing enabled chains")?;
    anyhow::ensure!(!chains.is_empty(), "no enabled chains — nothing to check");

    // What has actually been PUBLISHED. Read from the committed index, not
    // from the database: a run row proves a sweep happened, and this is
    // supposed to attest that its data reached the public bucket. Those are
    // different claims and only one of them is what readers depend on.
    let published: HashSet<String> = std::fs::read_to_string("published-runs.json")
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            v.as_array().map(|rs| {
                rs.iter()
                    .filter_map(|r| r["run_id"].as_str().map(str::to_string))
                    .collect()
            })
        })
        .unwrap_or_default();

    let mut stale: Vec<String> = Vec::new();
    for chain in &chains {
        let newest: Option<(uuid::Uuid, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            "SELECT run_id, finished_at FROM runs \
             WHERE chain = $1 AND status = 'finished' AND finished_at IS NOT NULL \
             ORDER BY finished_at DESC LIMIT 1",
        )
        .bind(chain)
        .fetch_optional(&db.pool)
        .await
        .with_context(|| format!("finding the newest run for {chain}"))?;

        match newest {
            None => stale.push(format!("{chain}: never swept")),
            Some((run_id, finished_at)) => {
                let age = (chrono::Utc::now() - finished_at).num_days();
                if age > max_age {
                    stale.push(format!("{chain}: newest finished run is {age} days old"));
                } else if !published.contains(&run_id.to_string()) {
                    // Swept but not published. The sweep worked and the
                    // publication step did not, which from a reader's side is
                    // the same as the sweep not having happened.
                    stale.push(format!(
                        "{chain}: run {} finished {age}d ago but is NOT in published-runs.json",
                        &run_id.to_string()[..8]
                    ));
                } else {
                    tracing::info!("{chain}: healthy — published, {age}d old");
                }
            }
        }
    }

    if !stale.is_empty() {
        for s in &stale {
            tracing::error!("STALE {s}");
        }
        // No ping. The whole mechanism is that the monitor alerts on SILENCE,
        // so the correct action when something is wrong is to say nothing to
        // it — and exit non-zero so this run is also visible on its own terms.
        anyhow::bail!(
            "{} of {} chains stale (max age {max_age}d) — heartbeat NOT sent",
            stale.len(),
            chains.len()
        );
    }

    match std::env::var("HEARTBEAT_URL") {
        Err(_) => {
            // Not a warning: this is the intended default. The self-check
            // above already passed, which is the part that catches partial
            // publication. See the module doc for when the ping is worth
            // turning on.
            tracing::info!(
                "all {} chains healthy and published. No HEARTBEAT_URL set, so \
                 nothing was pinged — the scheduler not firing at all is caught \
                 by whoever expects this week's report, not by this.",
                chains.len()
            );
        }
        Ok(url) => {
            let res = reqwest::Client::new()
                .get(&url)
                .timeout(std::time::Duration::from_secs(15))
                .send()
                .await
                .context("pinging the heartbeat monitor")?;
            anyhow::ensure!(
                res.status().is_success(),
                "heartbeat monitor answered {}",
                res.status()
            );
            tracing::info!("all {} chains healthy — heartbeat sent", chains.len());
        }
    }
    Ok(())
}
