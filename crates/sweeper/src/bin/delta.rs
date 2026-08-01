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
//! An agent present in one run and not the other is a population change, never
//! a status transition. Folding arrivals into "changed status" would make the
//! largest flip in every sweep be new registrations, which says nothing.
//!
//! A rung that is absent for an agent in one run and present in the other is
//! also not a transition. That is how rung 6 shipping would otherwise appear:
//! 27,956 agents "flipping" from nothing to a status, which is a fact about
//! this project rather than about them. Both sides must have a row for the
//! same rung before a flip is recorded.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::json;
use sweeper::store;
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

    let agents_after: std::collections::HashSet<u64> = after.keys().map(|(a, _)| *a).collect();
    let agents_before: std::collections::HashSet<u64> = before.keys().map(|(a, _)| *a).collect();

    let newly_registered = agents_after.difference(&agents_before).count();
    let disappeared = agents_before.difference(&agents_after).count();

    // Per-rung transitions, counted only where BOTH runs have a row for that
    // (agent, rung). See the module doc for why absence is never a flip.
    let mut flips: HashMap<(i16, String, String), i64> = HashMap::new();
    for ((agent, rung), new_status) in &after {
        let Some(old_status) = before.get(&(*agent, *rung)) else {
            continue;
        };
        if old_status == new_status {
            continue;
        }
        *flips
            .entry((*rung, old_status.clone(), new_status.clone()))
            .or_insert(0) += 1;
    }

    // Rung 2 called out on its own, because it carries a published series.
    // Derived from the same `flips` data rather than counted separately, so
    // the headline number and the table underneath it cannot disagree.
    let newly_resolving: i64 = flips
        .iter()
        .filter(|((rung, from, to), _)| *rung == 2 && from != "pass" && to == "pass")
        .map(|(_, n)| *n)
        .sum();
    let stopped_resolving: i64 = flips
        .iter()
        .filter(|((rung, from, to), _)| *rung == 2 && from == "pass" && to != "pass")
        .map(|(_, n)| *n)
        .sum();

    let mut flip_rows: Vec<_> = flips
        .into_iter()
        .map(|((rung, from, to), agents)| json!({"rung": rung, "from": from, "to": to, "agents": agents}))
        .collect();
    // Deterministic order, so two runs of this binary produce byte-identical
    // JSON and a diff of the stored row means something changed in the data.
    flip_rows.sort_by_key(|v| {
        (
            v["rung"].as_i64().unwrap_or(0),
            v["from"].as_str().unwrap_or("").to_string(),
            v["to"].as_str().unwrap_or("").to_string(),
        )
    });

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
        agents_before: agents_before.len() as i32,
        agents_after: agents_after.len() as i32,
        newly_registered: newly_registered as i32,
        disappeared: disappeared as i32,
        newly_resolving: newly_resolving as i32,
        stopped_resolving: stopped_resolving as i32,
        flips: &serde_json::Value::Array(flip_rows),
        checker_before: &checker_before,
        checker_after: &checker_after,
        schema_before,
        schema_after,
    })
    .await?;

    tracing::info!(
        "delta written: {} agents (was {}), +{} registered, -{} disappeared, \
         +{} resolving, -{} STOPPED resolving",
        agents_after.len(),
        agents_before.len(),
        newly_registered,
        disappeared,
        newly_resolving,
        stopped_resolving,
    );
    Ok(())
}
