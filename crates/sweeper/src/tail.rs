//! The continuous registration tail — what the registry contains that the last
//! census has not yet asked a single question about.
//!
//! ## The problem this exists for
//!
//! The census pins a block: "N agents as they existed simultaneously at block
//! X". That pin is the whole authority of the number, and it means an agent
//! minted five minutes after the sweep is, to this site, invisible — searching
//! finds nothing and its permalink 404s. The person most likely to look is the
//! registrant themselves, and what they learn is that the site is broken.
//!
//! On-chain discovery is cheap: ids are contiguous from 0, so
//! [`chain::Registry::highest_agent_id`] finds the top of the range in
//! ~O(log n) `ownerOf` calls, and reading a specific id is two more. What is
//! expensive is everything after that — fetching the declared document,
//! probing endpoints, judging seven rungs. So this module does the cheap half
//! continuously and the expensive half never.
//!
//! ## The boundary, and where it is enforced
//!
//! `registration_tail` (migration 0018) has NO `run_id` and NO foreign key to
//! `runs`. Every census aggregate — every rate, finding, delta and archive —
//! starts from `runs` and joins downward, so tail rows are not reachable from
//! those queries at all. The separation is structural: it cannot be broken by
//! forgetting a `WHERE`, because there is no join that would reach these rows
//! in the first place.
//!
//! Nothing in this module writes to `agent_snapshots`, `check_results`,
//! `http_archive`, `agent_documents` or `runs.agent_count`. The one write it
//! makes outside its own two tables is none: [`Db::supersede_tail`] writes only
//! `registration_tail.superseded_by_run`, reading `agent_snapshots` to decide
//! which rows the census has caught up with.
//!
//! ## What a tail row means, and what it does not
//!
//! "The registry contained this id at this block, owned by this address,
//! declaring this URI." That is a receipt, not a measurement. It carries no
//! rung, no status, no evidence — and the API surface is shaped so a client
//! cannot render one (see `api::routes::tail`).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::store::{Db, escape_nuls_for_postgres};

/// What one poll tick should do for one chain.
///
/// Pure — computed from three numbers, so the decision that governs how much
/// work a tick does is testable without a database or an RPC endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailPlan {
    /// The ids to read this tick, ascending and contiguous. Empty when there
    /// is nothing new (the ordinary case: most ticks find nothing).
    pub ids: Vec<u64>,
    /// What `registration_tail_cursor.highest_agent_id` should become IF every
    /// id in `ids` is read and stored. `None` means "leave the cursor alone".
    ///
    /// Never higher than the last id actually planned, so a capped tick
    /// resumes exactly where it stopped rather than skipping the remainder.
    pub next_cursor: Option<u64>,
    /// How many ids the cap deferred to a later tick. Non-zero is not an
    /// error — it is the cap doing its job — but it is worth logging, because
    /// a value that stays non-zero for many ticks means the poll interval and
    /// the cap are jointly too small for the chain's mint rate.
    pub remaining: u64,
}

/// Which ids are new, given where the tail resumed from and what the chain
/// says now.
///
/// `highest_known` is the highest id already accounted for — the greater of
/// the tail's own cursor and the highest id the last finished census run
/// swept. `highest_on_chain` is [`chain::Registry::highest_agent_id`] at this
/// tick's block.
///
/// Three cases deserve their names:
///
/// * **An empty registry** (`highest_on_chain` is `None`): nothing to do, and
///   the cursor is not invented — there is no id to record.
/// * **No baseline at all** (`highest_known` is `None`, i.e. this chain has
///   never been swept and the tail has never polled): the cursor is adopted at
///   the current head and NO rows are written. A tail is defined relative to a
///   census; backfilling 60,000 unchecked ids because no census exists yet
///   would build a shadow census with no checks in it, which is the one thing
///   this table must never become.
/// * **The cursor is ahead of the chain** (a reorg, or a census swept at a
///   later block than the tail last polled): the cursor never moves backwards,
///   and nothing is read. Re-reading ids already stored would be harmless
///   (the insert is `ON CONFLICT DO NOTHING`) but pointless.
pub fn plan_new_ids(
    highest_known: Option<u64>,
    highest_on_chain: Option<u64>,
    cap: usize,
) -> TailPlan {
    let Some(head) = highest_on_chain else {
        return TailPlan {
            ids: Vec::new(),
            next_cursor: None,
            remaining: 0,
        };
    };
    let Some(known) = highest_known else {
        return TailPlan {
            ids: Vec::new(),
            next_cursor: Some(head),
            remaining: 0,
        };
    };
    if known >= head {
        return TailPlan {
            ids: Vec::new(),
            next_cursor: Some(known),
            remaining: 0,
        };
    }

    let ids: Vec<u64> = (known + 1..=head).take(cap).collect();
    // `ids` is non-empty here: `known < head` guarantees at least one id, and
    // a cap of 0 is normalised away by the caller (see `tail_cap`).
    let last = ids.last().copied().unwrap_or(known);
    TailPlan {
        ids,
        next_cursor: Some(last),
        remaining: head - last,
    }
}

/// One tail row, as the API and the poller read it back.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TailRow {
    pub chain: String,
    pub agent_id: i64,
    pub owner: String,
    pub agent_uri: String,
    pub discovery_block: i64,
    pub discovered_at: DateTime<Utc>,
}

impl Db {
    /// The highest agent id the last FINISHED census run on this chain swept.
    ///
    /// Finished only: an in-flight run's rows are still arriving, and treating
    /// its partial coverage as a baseline would make the tail skip the ids
    /// that run has not reached yet — the exact agents the tail exists for.
    ///
    /// Scoped by `run_id` (the `agent_snapshots` primary key's leading column)
    /// rather than by chain across all runs, so this is an index lookup rather
    /// than a scan of every snapshot ever written.
    pub async fn census_high_water(&self, chain: &str) -> Result<Option<u64>> {
        let run: Option<(Uuid,)> = sqlx::query_as(
            "SELECT run_id FROM runs \
             WHERE chain = $1 AND status = 'finished' AND finished_at IS NOT NULL \
             ORDER BY finished_at DESC LIMIT 1",
        )
        .bind(chain)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("finding the newest finished run for {chain}"))?;
        let Some((run_id,)) = run else {
            return Ok(None);
        };
        let high: Option<i64> =
            sqlx::query_scalar("SELECT max(agent_id) FROM agent_snapshots WHERE run_id = $1")
                .bind(run_id)
                .fetch_one(&self.pool)
                .await
                .with_context(|| format!("reading the census high-water mark for {chain}"))?;
        Ok(high.map(|h| h.max(0) as u64))
    }

    /// Where the tail resumed from last time, and at which block.
    pub async fn tail_cursor(&self, chain: &str) -> Result<Option<(u64, u64)>> {
        let row: Option<(i64, i64)> = sqlx::query_as(
            "SELECT highest_agent_id, last_block FROM registration_tail_cursor WHERE chain = $1",
        )
        .bind(chain)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("reading the tail cursor for {chain}"))?;
        Ok(row.map(|(id, block)| (id.max(0) as u64, block.max(0) as u64)))
    }

    /// Record one discovered agent.
    ///
    /// `ON CONFLICT DO NOTHING` on the primary key `(chain, agent_id)`, which
    /// is what makes the whole poller idempotent: re-running it, or running two
    /// of them, inserts nothing new and cannot rewrite what an earlier poll
    /// saw. The FIRST sighting is the one kept — `discovered_at` is a claim
    /// about when this registry first showed us the id, and a later poll must
    /// not move it.
    ///
    /// Returns `true` when a row was actually inserted, so a tick can log how
    /// much it genuinely found rather than how many ids it looked at.
    pub async fn record_tail(&self, chain: &str, s: &chain::AgentSnapshot) -> Result<bool> {
        // The same on-chain-controlled-string hazard `store::insert_snapshot`
        // guards against: Postgres refuses a literal NUL in TEXT, and agent
        // 16791's `tokenURI()` on Base really does contain one. Escaped
        // losslessly, exactly as the census escapes it, so the two tables
        // cannot disagree about what the chain returned.
        let agent_uri = escape_nuls_for_postgres(s.agent_id, &s.agent_uri);
        let result = sqlx::query(
            "INSERT INTO registration_tail \
               (chain, agent_id, token_id, owner, agent_uri, discovery_block) \
             VALUES ($1,$2,$3::numeric,$4,$5,$6) \
             ON CONFLICT (chain, agent_id) DO NOTHING",
        )
        .bind(chain)
        .bind(s.agent_id as i64)
        .bind(s.token_id.to_string())
        .bind(&s.owner)
        .bind(agent_uri.as_ref())
        .bind(s.block_number as i64)
        .execute(&self.pool)
        .await
        .with_context(|| format!("recording tail row for {chain}/{}", s.agent_id))?;
        Ok(result.rows_affected() > 0)
    }

    /// Move the cursor forward. Never backward: `GREATEST` in SQL rather than
    /// a read-compare-write in Rust, so two pollers racing cannot make the
    /// tail forget ids it has already read.
    pub async fn advance_tail_cursor(
        &self,
        chain: &str,
        highest_agent_id: u64,
        last_block: u64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO registration_tail_cursor (chain, highest_agent_id, last_block) \
             VALUES ($1,$2,$3) \
             ON CONFLICT (chain) DO UPDATE SET \
               highest_agent_id = GREATEST(registration_tail_cursor.highest_agent_id, \
                                           EXCLUDED.highest_agent_id), \
               last_block = GREATEST(registration_tail_cursor.last_block, \
                                     EXCLUDED.last_block), \
               polled_at = now()",
        )
        .bind(chain)
        .bind(highest_agent_id as i64)
        .bind(last_block as i64)
        .execute(&self.pool)
        .await
        .with_context(|| format!("advancing the tail cursor for {chain}"))?;
        Ok(())
    }

    /// Mark every tail row this run has now swept.
    ///
    /// Called when a run is closed (see [`Db::close_run`]), and again by the
    /// poller as a backstop. The row is not deleted: it records when an agent
    /// FIRST appeared on chain, which the census — a series of pinned
    /// snapshots — cannot say. `superseded_by_run` only records that the tail
    /// is done showing it.
    ///
    /// The direction of the join matters. It reads `agent_snapshots` to decide
    /// which tail rows to mark, and writes only `registration_tail`. Nothing
    /// flows the other way, and nothing here changes any census figure.
    ///
    /// Returns how many rows were marked.
    pub async fn supersede_tail(&self, run_id: Uuid) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE registration_tail t SET superseded_by_run = $1 \
             FROM agent_snapshots s \
             WHERE s.run_id = $1 AND s.chain = t.chain AND s.agent_id = t.agent_id \
               AND t.superseded_by_run IS NULL",
        )
        .bind(run_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("superseding tail rows covered by run {run_id}"))?;
        Ok(result.rows_affected())
    }

    /// Every chain the census is configured to sweep.
    ///
    /// From the `chains` table rather than a list in code, for the same reason
    /// `heartbeat` reads it there: a chain enabled in the database and missing
    /// from a hardcoded list would silently have no tail at all.
    pub async fn enabled_chains(&self) -> Result<Vec<String>> {
        sqlx::query_scalar("SELECT chain FROM chains WHERE enabled ORDER BY chain")
            .fetch_all(&self.pool)
            .await
            .context("listing enabled chains")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ordinary tick: nothing has been minted since the last poll. The
    /// cursor stays put and no id is read — this is what almost every tick
    /// does, and it must cost nothing beyond the head read.
    #[test]
    fn nothing_new_reads_nothing_and_leaves_the_cursor_where_it_is() {
        let plan = plan_new_ids(Some(60_000), Some(60_000), 500);
        assert!(plan.ids.is_empty());
        assert_eq!(plan.next_cursor, Some(60_000));
        assert_eq!(plan.remaining, 0);
    }

    /// Three mints since the last poll: exactly the three ids above the
    /// cursor, ascending, and the cursor lands on the last of them.
    #[test]
    fn only_ids_above_the_cursor_are_new() {
        let plan = plan_new_ids(Some(60_000), Some(60_003), 500);
        assert_eq!(plan.ids, vec![60_001, 60_002, 60_003]);
        assert_eq!(plan.next_cursor, Some(60_003));
        assert_eq!(plan.remaining, 0);
    }

    /// **The bounded-work guarantee.** A burst mint of 10,000 agents must not
    /// make one tick run for an hour: the tick takes `cap` ids, the cursor
    /// advances only that far, and the rest is reported as remaining rather
    /// than silently skipped.
    #[test]
    fn a_burst_mint_is_capped_and_the_remainder_is_deferred_not_skipped() {
        let plan = plan_new_ids(Some(1_000), Some(11_000), 500);
        assert_eq!(plan.ids.len(), 500);
        assert_eq!(plan.ids.first(), Some(&1_001));
        assert_eq!(plan.ids.last(), Some(&1_500));
        assert_eq!(plan.next_cursor, Some(1_500));
        assert_eq!(plan.remaining, 9_500);

        // The next tick resumes at exactly the id the capped one stopped
        // before — no gap, no repeat.
        let next = plan_new_ids(plan.next_cursor, Some(11_000), 500);
        assert_eq!(next.ids.first(), Some(&1_501));
    }

    /// A chain with no finished census run and no cursor adopts the current
    /// head and writes nothing. A tail is defined relative to a census;
    /// backfilling the whole registry as unchecked rows would make this table
    /// a shadow census.
    #[test]
    fn a_chain_with_no_census_adopts_the_head_and_records_no_rows() {
        let plan = plan_new_ids(None, Some(59_997), 500);
        assert!(plan.ids.is_empty());
        assert_eq!(plan.next_cursor, Some(59_997));
    }

    /// An empty registry invents no cursor: there is no id to record.
    #[test]
    fn an_empty_registry_does_nothing_at_all() {
        assert_eq!(
            plan_new_ids(None, None, 500),
            TailPlan {
                ids: Vec::new(),
                next_cursor: None,
                remaining: 0
            }
        );
        assert_eq!(plan_new_ids(Some(42), None, 500).next_cursor, None);
    }

    /// The cursor never moves backwards. A census that swept further than the
    /// tail has polled (or a reorg that shortened the registry) must not make
    /// the tail re-read — or worse, re-claim — ids below where it already is.
    #[test]
    fn the_cursor_never_moves_backwards() {
        let plan = plan_new_ids(Some(60_010), Some(60_000), 500);
        assert!(plan.ids.is_empty());
        assert_eq!(plan.next_cursor, Some(60_010));
    }

    /// A cap larger than the gap is not padded: the plan holds exactly the
    /// ids that exist.
    #[test]
    fn a_cap_wider_than_the_gap_takes_only_what_is_there() {
        let plan = plan_new_ids(Some(7), Some(9), 500);
        assert_eq!(plan.ids, vec![8, 9]);
        assert_eq!(plan.remaining, 0);
    }
}
