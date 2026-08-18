//! Recomputing stored deltas from archived results.
//!
//! Shared by `backfill-refused` (which changes what rows say and must then
//! refresh what the deltas derived from them) and `recompute-deltas` (which
//! refreshes deltas after the ARITHMETIC changes, as on 2026-08-18 when
//! rung-2 `error` transitions were excluded from the headline series). A
//! delta is a published series; two implementations of its refresh would
//! eventually be two answers, which is the same argument that put
//! [`crate::delta::compute`] in one place.
//!
//! This reads only what the database already holds — no network request is
//! made and no chain is read. A delta is derived, not measured, which is why
//! recomputing it is legitimate at all; the measurements it derives from are
//! never touched here.

use anyhow::Result;
use uuid::Uuid;

use crate::{delta, store};

/// One stored delta row, before and after the recomputation.
pub struct Recomputed {
    pub run_id: Uuid,
    pub chain: String,
    /// The two headline series as the row stored them before this pass.
    pub stored_newly_resolving: i32,
    pub stored_stopped_resolving: i32,
    /// What `delta::compute` says today, from the same archived statuses.
    pub counts: delta::DeltaCounts,
}

/// Recompute every `run_deltas` row through the one implementation of the
/// arithmetic. Writes only when `apply`; a dry run returns the same report
/// without touching a row.
pub async fn all(db: &store::Db, apply: bool) -> Result<Vec<Recomputed>> {
    let mut out = Vec::new();
    for (new_run, old_run, chain, stored_newly, stored_stopped) in db.all_deltas().await? {
        let after = db.rung_statuses(new_run).await?;
        let before = db.rung_statuses(old_run).await?;
        let counts = delta::compute(&before, &after);
        if apply {
            let (checker_after, schema_after) = db.run_provenance(new_run).await?;
            let (checker_before, schema_before) = db.run_provenance(old_run).await?;
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
        }
        out.push(Recomputed {
            run_id: new_run,
            chain,
            stored_newly_resolving: stored_newly,
            stored_stopped_resolving: stored_stopped,
            counts,
        });
    }
    Ok(out)
}
