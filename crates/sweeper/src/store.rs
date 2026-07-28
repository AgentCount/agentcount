//! Persistence for one run. Everything here is INSERT-only: a run's results
//! are never updated, because a changed result with the same run_id would
//! make the archive lie.

use std::borrow::Cow;
use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

pub struct Db {
    pub pool: PgPool,
}

pub struct RunMeta {
    pub run_id: Uuid,
    pub chain: String,
    /// The block every read in this run is pinned to. Stored on `runs` (as of
    /// migration 0009) so a resumed session can read the remainder at the
    /// SAME block the first session used, rather than re-pinning to whatever
    /// block happens to be current when the resume starts.
    pub pinned_block: u64,
    pub schema_version: i32,
    pub checker_version: String,
    pub checker_commit: String,
    pub spec_commit: String,
    pub rerun_command: String,
}

/// Everything needed to resume an existing run: its provenance, reloaded from
/// `runs` rather than regenerated, plus the pinned block reads must stay
/// locked to.
pub struct ResumedRun {
    pub chain: String,
    pub pinned_block: u64,
    pub schema_version: i32,
    pub checker_version: String,
    pub checker_commit: String,
    pub spec_commit: String,
    pub rerun_command: String,
    pub started_at: DateTime<Utc>,
}

/// Postgres `TEXT` (and `JSONB`) refuse a literal NUL byte outright —
/// `invalid byte sequence for encoding "UTF8": 0x00` — and this is real:
/// agent 16791's `tokenURI()` on Base returns a `data:` URI with an embedded
/// NUL. Silently dropping the byte (as `crates/enricher`'s `strip_nuls` does
/// for fetched card bodies) would make the stored value diverge from what the
/// chain actually returned, which is exactly the kind of quiet substitution
/// this project exists not to do for the values it persists as fact.
///
/// Instead, escape each NUL as the six-character sequence a backslash
/// followed by the four digits `0000` — the same escape JSON itself would use
/// for the character (see [`escape_nuls_for_postgres`]'s own implementation
/// below for the exact literal). That is lossless (a reader can reconstruct
/// the exact on-chain bytes by reversing the substitution) and safe for
/// `TEXT`. Returns the input unchanged (no allocation) when there is nothing
/// to escape.
///
/// Deliberately placed in `crates/sweeper`, not `crates/chain`: a
/// `chain::AgentSnapshot.agent_uri` must keep the true bytes so the export and
/// any future consumer see reality. Only the database write — the boundary
/// where Postgres's constraint actually bites — escapes.
fn escape_nuls_for_postgres(agent_id: u64, uri: &str) -> Cow<'_, str> {
    if !uri.contains('\0') {
        return Cow::Borrowed(uri);
    }
    tracing::warn!(
        "agent {agent_id}: agent_uri contains a NUL byte — escaping as \\u0000 before writing to Postgres"
    );
    Cow::Owned(uri.replace('\0', "\\u0000"))
}

impl Db {
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(url)
            .await
            .context("connecting to Postgres")?;
        Ok(Self { pool })
    }

    pub async fn open_run(&self, m: &RunMeta) -> Result<()> {
        sqlx::query(
            "INSERT INTO runs (run_id, chain, pinned_block, schema_version, checker_version, \
                               checker_commit, spec_commit, rerun_command) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(m.run_id)
        .bind(&m.chain)
        .bind(m.pinned_block as i64)
        .bind(m.schema_version)
        .bind(&m.checker_version)
        .bind(&m.checker_commit)
        .bind(&m.spec_commit)
        .bind(&m.rerun_command)
        .execute(&self.pool)
        .await
        .context("opening run")?;
        Ok(())
    }

    /// Reload an existing run's provenance and pinned block so a resumed
    /// sweep can reuse them instead of opening a new run. Errors if the run
    /// doesn't exist, or if it predates migration 0009 and has no
    /// `pinned_block` recorded (nothing safe to resume against).
    pub async fn load_run(&self, run_id: Uuid) -> Result<ResumedRun> {
        let row: (
            String,
            Option<i64>,
            i32,
            String,
            String,
            String,
            String,
            DateTime<Utc>,
        ) = sqlx::query_as(
            "SELECT chain, pinned_block, schema_version, checker_version, \
                        checker_commit, spec_commit, rerun_command, started_at \
                 FROM runs WHERE run_id = $1",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("no run {run_id} to resume"))?;
        let pinned_block = row.1.with_context(|| {
            format!(
                "run {run_id} has no pinned_block recorded — it predates migration 0009 \
                 and cannot be resumed"
            )
        })?;
        Ok(ResumedRun {
            chain: row.0,
            pinned_block: pinned_block as u64,
            schema_version: row.2,
            checker_version: row.3,
            checker_commit: row.4,
            spec_commit: row.5,
            rerun_command: row.6,
            started_at: row.7,
        })
    }

    /// Every agent id already snapshotted for this run — the resume set to
    /// skip. Scoped by run_id AND chain (a run is always single-chain, but
    /// the extra predicate costs nothing and matches how every other query
    /// here addresses `agent_snapshots`).
    pub async fn swept_agent_ids(&self, run_id: Uuid, chain: &str) -> Result<HashSet<u64>> {
        let rows: Vec<(i64,)> =
            sqlx::query_as("SELECT agent_id FROM agent_snapshots WHERE run_id = $1 AND chain = $2")
                .bind(run_id)
                .bind(chain)
                .fetch_all(&self.pool)
                .await
                .context("loading already-swept agent ids")?;
        Ok(rows.into_iter().map(|(id,)| id as u64).collect())
    }

    pub async fn write_snapshot(
        &self,
        run_id: Uuid,
        chain: &str,
        s: &chain::AgentSnapshot,
    ) -> Result<()> {
        let agent_uri = escape_nuls_for_postgres(s.agent_id, &s.agent_uri);
        sqlx::query(
            "INSERT INTO agent_snapshots \
               (run_id, chain, agent_id, token_id, owner, agent_uri, block_number) \
             VALUES ($1,$2,$3,$4::numeric,$5,$6,$7)",
        )
        .bind(run_id)
        .bind(chain)
        .bind(s.agent_id as i64)
        .bind(s.token_id.to_string())
        .bind(&s.owner)
        .bind(agent_uri.as_ref())
        .bind(s.block_number as i64)
        .execute(&self.pool)
        .await
        .context("writing snapshot")?;
        Ok(())
    }

    /// Persist what one agent's fetch actually returned — the durable record
    /// `http_archive` exists for. Written in the SAME per-agent unit as
    /// `write_snapshot`/`write_results` below (called right beside them in
    /// the sweeper's loop), never batched separately: a crash between the two
    /// must not leave a snapshot with no archive row or vice versa.
    ///
    /// `scheme` is the caller's ALREADY-NORMALISED bucket (`"empty"` |
    /// `"unsupported"` | `"data"` | `"http"` | `"https"` | `"ipfs"` — see
    /// `main::checks_scheme`), not `outcome.scheme` verbatim; the two can
    /// disagree for a malformed `data:`/`ipfs://` URI, and this column is
    /// documented (migration 0010) to hold the same six buckets rung 2 judges.
    pub async fn write_archive(
        &self,
        run_id: Uuid,
        chain: &str,
        agent_id: u64,
        requested_uri: &str,
        scheme: &str,
        outcome: &probe::FetchOutcome,
    ) -> Result<()> {
        // Same on-chain-controlled-string hazard `write_snapshot` guards
        // against: `requested_uri` is the identical `tokenURI()` value stored
        // there, so a raw NUL must be escaped the same lossless way before it
        // reaches this TEXT column.
        let requested_uri = escape_nuls_for_postgres(agent_id, requested_uri);
        let body_bytes = outcome.body.as_ref().map(|b| b.len() as i32);
        sqlx::query(
            "INSERT INTO http_archive \
               (run_id, chain, agent_id, requested_uri, scheme, request_url, final_url, \
                http_status, content_type, headers, body, body_bytes, body_sha256, \
                truncated, error, elapsed_ms) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)",
        )
        .bind(run_id)
        .bind(chain)
        .bind(agent_id as i64)
        .bind(requested_uri.as_ref())
        .bind(scheme)
        .bind(&outcome.request_url)
        .bind(&outcome.final_url)
        .bind(outcome.http_status.map(i32::from))
        .bind(&outcome.content_type)
        .bind(&outcome.headers)
        .bind(outcome.body.as_deref())
        .bind(body_bytes)
        .bind(&outcome.body_sha256)
        .bind(outcome.truncated)
        .bind(&outcome.error)
        .bind(outcome.elapsed_ms.map(|ms| ms as i32))
        .execute(&self.pool)
        .await
        .context("writing http archive")?;
        Ok(())
    }

    pub async fn write_results(
        &self,
        run_id: Uuid,
        chain: &str,
        agent_id: u64,
        results: &[checks::CheckResult],
    ) -> Result<()> {
        for r in results {
            sqlx::query(
                "INSERT INTO check_results \
                   (run_id, chain, agent_id, rung, name, status, evidence, checked_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            )
            .bind(run_id)
            .bind(chain)
            .bind(agent_id as i64)
            .bind(r.rung as i16)
            .bind(r.name)
            .bind(r.status.as_str())
            .bind(&r.evidence)
            .bind(r.checked_at)
            .execute(&self.pool)
            .await
            .context("writing check result")?;
        }
        Ok(())
    }

    pub async fn close_run(&self, run_id: Uuid, agent_count: i32, at: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE runs SET finished_at = $2, agent_count = $3 WHERE run_id = $1")
            .bind(run_id)
            .bind(at)
            .bind(agent_count)
            .execute(&self.pool)
            .await
            .context("closing run")?;
        Ok(())
    }

    /// The registry address and chain id for a chain, from the `chains` table.
    pub async fn chain_config(&self, chain: &str) -> Result<(i64, String, i64)> {
        let row: (i64, String, i64) = sqlx::query_as(
            "SELECT chain_id, identity_registry, deploy_block FROM chains \
             WHERE chain = $1 AND enabled",
        )
        .bind(chain)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("no enabled chain named {chain}"))?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The literal on-chain hazard this whole fix responds to: agent 16791's
    /// real `tokenURI()` on Base is a `data:application/json;base64,...`
    /// string with a raw NUL embedded in it. A string with no NUL at all must
    /// pass through untouched (and without allocating — `Cow::Borrowed`).
    #[test]
    fn a_uri_without_a_nul_is_returned_unchanged() {
        let uri = "data:application/json;base64,eyJuYW1lIjoiYWdlbnQifQ==";
        let escaped = escape_nuls_for_postgres(1, uri);
        assert_eq!(escaped.as_ref(), uri);
        assert!(matches!(escaped, Cow::Borrowed(_)));
    }

    /// The core guarantee: escaping is lossless and reversible. A reader who
    /// splits the stored value on the six-character escape sequence and
    /// substitutes a real NUL back in reconstructs EXACTLY the bytes the
    /// chain returned — nothing invented, nothing dropped.
    #[test]
    fn a_nul_byte_round_trips_through_the_escape() {
        let original = "data:application/json;base64,eyJhIjoxfQ==\0trailing-garbage-after-nul";
        let escaped = escape_nuls_for_postgres(16791, original);

        // The stored form must contain no raw NUL — that's the whole point,
        // it's what Postgres rejects.
        assert!(!escaped.contains('\0'));

        // And it must be reconstructible back to the original bytes exactly.
        let roundtripped = escaped.replace("\\u0000", "\0");
        assert_eq!(roundtripped, original);
    }

    /// Multiple NULs in one URI must each survive the round trip
    /// independently, not just the first one.
    #[test]
    fn multiple_nuls_all_round_trip() {
        let original = "\0first\0second\0";
        let escaped = escape_nuls_for_postgres(2, original);
        assert!(!escaped.contains('\0'));
        assert_eq!(escaped.replace("\\u0000", "\0"), original);
    }
}
