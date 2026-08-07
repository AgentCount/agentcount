//! Persistence for one run. Everything here is INSERT-only: a run's results
//! are never updated, because a changed result with the same run_id would
//! make the archive lie.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// `Clone` is cheap: `PgPool` is an `Arc` internally, so a clone shares the
/// same connection pool. The stall watchdog needs its own handle because it
/// must be able to reach the database while the main task is wedged.
#[derive(Clone)]
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

/// Everything [`Db::write_agent`] needs for one agent's three rows, bundled
/// (same reason as [`RunMeta`] above) so the method's signature doesn't grow
/// an argument every time a caller needs to thread one more piece of
/// evidence through.
pub struct AgentWrite<'a> {
    pub run_id: Uuid,
    pub chain: &'a str,
    pub snapshot: &'a chain::AgentSnapshot,
    /// The RAW `tokenURI()` value — [`Db::insert_snapshot`] and
    /// [`Db::insert_archive`] each escape any embedded NUL themselves before
    /// their own write, same as before this struct existed.
    pub requested_uri: &'a str,
    /// The caller's already-normalised scheme bucket (`main::checks_scheme`),
    /// not `outcome.scheme` verbatim — see [`Db::insert_archive`]'s doc.
    pub scheme: &'a str,
    pub outcome: &'a probe::FetchOutcome,
    pub results: &'a [checks::CheckResult],
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
///
/// **Public because TEXT is not the only boundary.** A NUL also cannot
/// survive a `jsonb` column: Postgres accepts `\0` in `json` but rejects
/// it in `jsonb`, which is the type `check_results.evidence` uses. Rung 2's
/// evidence carries the agent's URI, so an unescaped NUL reaching a check
/// input fails the insert exactly as it fails a TEXT write — it just fails
/// later, after the snapshot row has already landed. Each binary therefore
/// escapes once, up front, and passes the escaped form to both the store and
/// the checks so the two can never disagree about what the value was.
///
/// Rung 6 needs the same guarantee for a different string: a declared
/// `services[].endpoint` is also an attacker-controlled value that reaches
/// both a TEXT column (`endpoint_probes.url`) and a `jsonb` one (rung 6's
/// per-entry evidence).
pub fn escape_nuls_for_postgres(agent_id: u64, uri: &str) -> Cow<'_, str> {
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
                               checker_commit, spec_commit, rerun_command, status, \
                               last_progress_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'running',now())",
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

    /// Insert one agent's snapshot row. Private: only ever called from
    /// within [`Db::write_agent`]'s transaction — see that method's doc for
    /// why the three writes below must never run outside one.
    async fn insert_snapshot(
        tx: &mut Transaction<'_, Postgres>,
        run_id: Uuid,
        chain: &str,
        s: &chain::AgentSnapshot,
    ) -> Result<(), sqlx::Error> {
        let agent_uri = escape_nuls_for_postgres(s.agent_id, &s.agent_uri);
        sqlx::query(
            "INSERT INTO agent_snapshots \
               (run_id, chain, agent_id, token_id, owner, agent_uri, block_number, \
                minter, registration_tx_hash, registration_block) \
             VALUES ($1,$2,$3,$4::numeric,$5,$6,$7,$8,$9,$10)",
        )
        .bind(run_id)
        .bind(chain)
        .bind(s.agent_id as i64)
        .bind(s.token_id.to_string())
        .bind(&s.owner)
        .bind(agent_uri.as_ref())
        .bind(s.block_number as i64)
        .bind(s.minter.as_ref())
        .bind(s.registration_tx_hash.as_ref())
        .bind(s.registration_block.map(|b| b as i64))
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Insert the durable record of what one agent's fetch actually returned.
    ///
    /// `scheme` is the caller's ALREADY-NORMALISED bucket (`"empty"` |
    /// `"unsupported"` | `"data"` | `"http"` | `"https"` | `"ipfs"` — see
    /// `main::checks_scheme`), not `outcome.scheme` verbatim; the two can
    /// disagree for a malformed `data:`/`ipfs://` URI, and this column is
    /// documented (migration 0010) to hold the same six buckets rung 2 judges.
    async fn insert_archive(
        tx: &mut Transaction<'_, Postgres>,
        run_id: Uuid,
        chain: &str,
        agent_id: u64,
        requested_uri: &str,
        scheme: &str,
        outcome: &probe::FetchOutcome,
    ) -> Result<(), sqlx::Error> {
        // Same on-chain-controlled-string hazard `insert_snapshot` guards
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
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn insert_results(
        tx: &mut Transaction<'_, Postgres>,
        run_id: Uuid,
        chain: &str,
        agent_id: u64,
        results: &[checks::CheckResult],
    ) -> Result<(), sqlx::Error> {
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
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    /// Persist ALL THREE of one agent's rows — the snapshot, the HTTP
    /// archive, and every rung's check result — in a single database
    /// transaction. Either the whole set lands or none of it does.
    ///
    /// This is the fix for a real incident: with three independent
    /// `?`-propagating inserts, a failure between them (the NUL-in-`jsonb`
    /// crash that took down two sweeps) could leave a snapshot row with no
    /// check results, silently breaking the invariant every run is verified
    /// against — `runs.agent_count`, `agent_snapshots`, `check_results`, and
    /// the exported file count must all agree. A transaction makes that
    /// invariant true by construction instead of by convention.
    ///
    /// Returns the raw [`sqlx::Error`] (not `anyhow`) so the caller —
    /// [`retry_transient`] and, above that, `main`'s per-agent loop — can
    /// inspect the SQLSTATE via [`classify_error`] before deciding whether to
    /// retry, log, and count the agent unwritable, or move on.
    ///
    /// Does NOT write the `data/<run_id>/` export file. That is a filesystem
    /// write and cannot join this transaction, so the caller must write it
    /// only after this method returns `Ok` — ordering, not atomicity, is what
    /// keeps an orphan export file from ever describing an agent the
    /// database rejected.
    pub async fn write_agent(&self, w: &AgentWrite<'_>) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        Self::insert_snapshot(&mut tx, w.run_id, w.chain, w.snapshot).await?;
        Self::insert_archive(
            &mut tx,
            w.run_id,
            w.chain,
            w.snapshot.agent_id,
            w.requested_uri,
            w.scheme,
            w.outcome,
        )
        .await?;
        Self::insert_results(&mut tx, w.run_id, w.chain, w.snapshot.agent_id, w.results).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Mark a run finished — and, as the same act, retire the tail rows it has
    /// now swept.
    ///
    /// **Why the supersede lives here.** "A census run finished covering this
    /// id" is exactly the event that makes a tail row stop being interesting,
    /// and this is the one place in the codebase where that event happens. Any
    /// other home (a cron job, a step in the weekly script) would be a second
    /// place that has to remember, and the failure mode of forgetting is the
    /// site showing an agent as "not yet checked" while its seven answers sit
    /// in the database.
    ///
    /// It is deliberately NOT in the same transaction, and a failure here is
    /// logged rather than propagated. The run's own status is a census fact
    /// and must land; whether a tail row still says "unswept" is a display
    /// detail that the poller's own backstop pass fixes on its next tick (see
    /// `bin/tail.rs`). Coupling them the other way would mean a missing
    /// migration 0018 could stop a completed sweep from ever being marked
    /// finished — a real census lost to a cosmetic table.
    pub async fn close_run(&self, run_id: Uuid, agent_count: i32, at: DateTime<Utc>) -> Result<()> {
        sqlx::query(
            "UPDATE runs SET finished_at = $2, agent_count = $3, status = 'finished' \
             WHERE run_id = $1",
        )
        .bind(run_id)
        .bind(at)
        .bind(agent_count)
        .execute(&self.pool)
        .await
        .context("closing run")?;

        match self.supersede_tail(run_id).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(
                "run {run_id}: {n} registration-tail row(s) superseded — those agents \
                 now have real check results and the tail stops showing them"
            ),
            Err(e) => tracing::warn!(
                "run {run_id} closed, but the registration tail could not be updated: {e:#} \
                 — the tail poller's backstop pass will retire those rows instead"
            ),
        }
        Ok(())
    }

    /// The registry address and chain id for a chain, from the `chains` table.
    ///
    /// `reputation_registry` is `None` exactly when the column is `NULL` —
    /// meaning this chain has no deployed Reputation Registry at all. Rung 7
    /// must read that as "we cannot check" (`Error`), never "the agent
    /// failed" (`Fail`) — see `checks::rung7_attested`'s module doc.
    pub async fn chain_config(&self, chain: &str) -> Result<(i64, String, Option<String>, i64)> {
        let row: (i64, String, Option<String>, i64) = sqlx::query_as(
            "SELECT chain_id, identity_registry, reputation_registry, deploy_block FROM chains \
             WHERE chain = $1 AND enabled",
        )
        .bind(chain)
        .fetch_one(&self.pool)
        .await
        .with_context(|| format!("no enabled chain named {chain}"))?;
        Ok(row)
    }
}

/// Whether a database write failure is worth retrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// Connection churn, pool exhaustion, a deadlock, the server shedding
    /// load under a serialization failure or "too many connections" — none
    /// of these say anything about the DATA being written. The identical
    /// write, tried again a moment later, may simply succeed.
    Transient,
    /// The data itself is the problem (the NUL-in-`jsonb` crash is SQLSTATE
    /// class `22`, data exception; a unique/foreign-key/not-null violation is
    /// class `23`) — or the code is one we don't recognise. Retrying sends
    /// the identical bytes into the identical wall five times instead of
    /// once; better to fail fast, log, and move to the next agent.
    Permanent,
}

/// Classify a write failure using `sqlx`'s TYPED error — never by matching on
/// the message string, which is prose and can change wording across Postgres
/// versions without changing meaning.
///
/// - [`sqlx::Error::Io`], [`sqlx::Error::PoolTimedOut`], and the pool/worker
///   variants below never reached a server (or lost the one they had), so
///   nothing about the agent's data caused them.
/// - [`sqlx::Error::Database`] carries the actual SQLSTATE the server
///   reported via [`sqlx::error::DatabaseError::code`]. Grouping by its
///   two-character CLASS (the first two digits — see the Postgres manual,
///   Appendix A, "PostgreSQL Error Codes") is the same granularity Postgres
///   itself uses to say "this whole family of conditions behaves alike":
///   `08` connection exception, `40` transaction rollback (includes
///   `40P01` deadlock and `40001` serialization failure), `53` insufficient
///   resources (includes `53300` too many connections), `57` operator
///   intervention (includes `57P03` cannot connect now / admin shutdown) are
///   all transient; everything else — notably `22` data exception and `23`
///   integrity constraint violation — is permanent, and so is a code we've
///   never seen.
/// - Every other variant (`RowNotFound`, `Protocol`, `ColumnDecode`, `Encode`,
///   `Decode`, `Configuration`, …) is a programming or mapping error, not a
///   transient condition, so it is permanent too.
pub fn classify_error(err: &sqlx::Error) -> Classification {
    match err {
        sqlx::Error::Io(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed => Classification::Transient,
        sqlx::Error::Database(db_err) => match db_err.code() {
            Some(code) if code.len() >= 2 => match &code[0..2] {
                "08" | "40" | "53" | "57" => Classification::Transient,
                _ => Classification::Permanent,
            },
            _ => Classification::Permanent,
        },
        _ => Classification::Permanent,
    }
}

/// How many times [`retry_transient`] will attempt an operation in total
/// (the first try plus up to 4 retries).
const MAX_ATTEMPTS: u32 = 5;

/// The first retry's delay. Doubles each attempt after that (200ms, 400ms,
/// 800ms, 1600ms — four sleeps between five attempts), so the worst case is
/// bounded at both ends: at most [`MAX_ATTEMPTS`] tries, and at most a few
/// seconds of total sleep. A persistently failing write can never stall the
/// run indefinitely.
const BASE_DELAY: Duration = Duration::from_millis(200);

/// Retry a fallible database operation with bounded exponential backoff —
/// but ONLY when [`classify_error`] says the failure is
/// [`Classification::Transient`]. A permanent error (bad data, a constraint
/// violation) is returned to the caller on the very first attempt: retrying
/// it would just spin, identically, up to `MAX_ATTEMPTS` times before giving
/// up anyway.
///
/// `op` is called fresh on every attempt (it's an `FnMut` returning a new
/// future each time) so it can, e.g., open a brand-new transaction each try —
/// a transaction that failed and rolled back cannot be reused.
pub async fn retry_transient<F, Fut, T>(mut op: F) -> Result<T, sqlx::Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, sqlx::Error>>,
{
    let mut attempt: u32 = 1;
    loop {
        match op().await {
            Ok(v) => return Ok(v),
            Err(err) => {
                if classify_error(&err) == Classification::Permanent || attempt >= MAX_ATTEMPTS {
                    return Err(err);
                }
                let delay = BASE_DELAY * 2u32.pow(attempt - 1);
                tracing::warn!(
                    "transient database error on attempt {attempt}/{MAX_ATTEMPTS}: {err:#} \
                     — retrying in {delay:?}"
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
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

/// Recursively escape NUL bytes inside a parsed JSON document, in both keys
/// and string values.
///
/// Postgres accepts a NUL escape in `json` but rejects it in `jsonb`, and
/// every evidence column in this schema is `jsonb`. A registration document
/// may legally carry one inside a string, which `serde_json` decodes to a real
/// NUL — so a document that parsed perfectly can still make an evidence insert
/// fail, aborting a multi-hour sweep *after* the agent's snapshot row has
/// already been written.
///
/// The substitution is the same lossless one [`escape_nuls_for_postgres`]
/// uses, so the original bytes stay reconstructable.
pub fn escape_nuls_in_json(v: &mut serde_json::Value) {
    use serde_json::Value;
    const ESCAPED: &str = "\\u0000";
    match v {
        Value::String(s) => {
            if s.contains('\0') {
                *s = s.replace('\0', ESCAPED);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(escape_nuls_in_json),
        Value::Object(map) => {
            for val in map.values_mut() {
                escape_nuls_in_json(val);
            }
            // Keys too: a NUL in a key fails the insert just as readily, and
            // rung 4 reports key names in `fields_found`/`fields_missing`.
            if map.keys().any(|k| k.contains('\0')) {
                let fixed: serde_json::Map<String, Value> = std::mem::take(map)
                    .into_iter()
                    .map(|(k, val)| (k.replace('\0', ESCAPED), val))
                    .collect();
                *map = fixed;
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod nul_json_tests {
    use super::*;

    #[test]
    fn nuls_are_escaped_in_keys_values_and_arrays() {
        // The JSON TEXT carries the escape sequence; serde decodes it into a
        // real NUL, which is exactly the shape that breaks a jsonb insert.
        let raw = "{\"na\\u0000me\": \"va\\u0000l\", \"arr\": [\"x\\u0000y\"]}";
        let mut v: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert!(
            v.to_string().contains('\u{0}') || raw.contains("\\u0000"),
            "precondition: the parsed document holds a real NUL"
        );

        escape_nuls_in_json(&mut v);

        // Values, keys, and array elements all carry the escape as literal
        // text now, and no raw NUL survives anywhere.
        assert_eq!(v["na\\u0000me"], "va\\u0000l");
        assert_eq!(v["arr"][0], "x\\u0000y");
        let mut any_nul = false;
        fn walk(v: &serde_json::Value, found: &mut bool) {
            match v {
                serde_json::Value::String(s) => *found |= s.contains('\u{0}'),
                serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, found)),
                serde_json::Value::Object(m) => m.iter().for_each(|(k, x)| {
                    *found |= k.contains('\u{0}');
                    walk(x, found);
                }),
                _ => {}
            }
        }
        walk(&v, &mut any_nul);
        assert!(!any_nul, "no raw NUL may survive anywhere in the document");
    }

    #[test]
    fn a_document_without_nuls_is_untouched() {
        let before: serde_json::Value = serde_json::json!({"name": "ok", "n": 1, "a": [true]});
        let mut after = before.clone();
        escape_nuls_in_json(&mut after);
        assert_eq!(before, after);
    }
}

#[cfg(test)]
mod classify_error_tests {
    use super::*;
    use sqlx::error::{DatabaseError, ErrorKind};
    use std::borrow::Cow;

    /// A minimal stand-in for `sqlx::postgres::PgDatabaseError`, which has no
    /// public constructor (its inner `Notice` is built only by decoding a
    /// real wire-protocol response). Stubbing the public `DatabaseError`
    /// trait lets these tests exercise `classify_error`'s real `Database`
    /// branch — including the SQLSTATE-class grouping — with an arbitrary
    /// code, with no live database involved.
    #[derive(Debug)]
    struct StubDbError {
        code: Option<&'static str>,
    }

    impl std::fmt::Display for StubDbError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "stub db error (code={:?})", self.code)
        }
    }

    impl std::error::Error for StubDbError {}

    impl DatabaseError for StubDbError {
        fn message(&self) -> &str {
            "stub"
        }
        fn code(&self) -> Option<Cow<'_, str>> {
            self.code.map(Cow::Borrowed)
        }
        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }
        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    fn db_error(code: Option<&'static str>) -> sqlx::Error {
        sqlx::Error::Database(Box::new(StubDbError { code }))
    }

    /// Class `08` — connection exception (e.g. `08006` connection failure).
    #[test]
    fn sqlstate_class_08_connection_exception_is_transient() {
        assert_eq!(
            classify_error(&db_error(Some("08006"))),
            Classification::Transient
        );
    }

    /// Class `40` — transaction rollback: `40001` serialization failure and
    /// `40P01` deadlock detected both live here.
    #[test]
    fn sqlstate_class_40_transaction_rollback_is_transient() {
        assert_eq!(
            classify_error(&db_error(Some("40001"))),
            Classification::Transient
        );
        assert_eq!(
            classify_error(&db_error(Some("40P01"))),
            Classification::Transient
        );
    }

    /// Class `53` — insufficient resources: `53300` too many connections.
    #[test]
    fn sqlstate_class_53_insufficient_resources_is_transient() {
        assert_eq!(
            classify_error(&db_error(Some("53300"))),
            Classification::Transient
        );
    }

    /// Class `57` — operator intervention: `57P03` cannot connect now /
    /// admin shutdown.
    #[test]
    fn sqlstate_class_57_operator_intervention_is_transient() {
        assert_eq!(
            classify_error(&db_error(Some("57P03"))),
            Classification::Transient
        );
    }

    /// Class `22` — data exception. This is the EXACT class the
    /// NUL-in-`jsonb` crash raised (`22P02`, "invalid text representation" /
    /// invalid byte sequence). Retrying sends the same bad bytes into the
    /// same wall.
    #[test]
    fn sqlstate_class_22_data_exception_is_permanent() {
        assert_eq!(
            classify_error(&db_error(Some("22P02"))),
            Classification::Permanent
        );
    }

    /// Class `23` — integrity constraint violation (unique / foreign key /
    /// not-null / check).
    #[test]
    fn sqlstate_class_23_integrity_constraint_violation_is_permanent() {
        assert_eq!(
            classify_error(&db_error(Some("23505"))),
            Classification::Permanent
        );
    }

    /// A code this classifier has never seen defaults to permanent, not
    /// transient — an unrecognised failure mode is not assumed safe to spin
    /// on.
    #[test]
    fn an_unrecognised_sqlstate_class_is_permanent() {
        assert_eq!(
            classify_error(&db_error(Some("99999"))),
            Classification::Permanent
        );
    }

    /// A database error with no SQLSTATE at all (some drivers/backends can
    /// omit it) is also treated as permanent — there is nothing here to say
    /// retrying is safe.
    #[test]
    fn a_database_error_with_no_code_is_permanent() {
        assert_eq!(classify_error(&db_error(None)), Classification::Permanent);
    }

    /// `Io`, `PoolTimedOut`, `PoolClosed`, and `WorkerCrashed` never reached
    /// (or lost) a server connection — nothing about the agent's data caused
    /// them, so all four are transient.
    #[test]
    fn connection_level_errors_are_transient() {
        assert_eq!(
            classify_error(&sqlx::Error::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "connection reset by peer"
            ))),
            Classification::Transient
        );
        assert_eq!(
            classify_error(&sqlx::Error::PoolTimedOut),
            Classification::Transient
        );
        assert_eq!(
            classify_error(&sqlx::Error::PoolClosed),
            Classification::Transient
        );
        assert_eq!(
            classify_error(&sqlx::Error::WorkerCrashed),
            Classification::Transient
        );
    }

    /// Programming/mapping errors are permanent: retrying a query that names
    /// a column which doesn't exist, or one whose row-shape decode failed,
    /// fails identically every time.
    #[test]
    fn programming_errors_are_permanent() {
        assert_eq!(
            classify_error(&sqlx::Error::RowNotFound),
            Classification::Permanent
        );
        assert_eq!(
            classify_error(&sqlx::Error::Protocol("garbled response".to_string())),
            Classification::Permanent
        );
        assert_eq!(
            classify_error(&sqlx::Error::ColumnNotFound("no_such_column".to_string())),
            Classification::Permanent
        );
    }
}

impl Db {
    /// Heartbeat. Called as agents land so a stalled run is visible from the
    /// database alone — the failure this exists for produced no error and no
    /// log line, only a process sitting at 0% CPU.
    pub async fn touch_progress(&self, run_id: Uuid) -> Result<()> {
        sqlx::query("UPDATE runs SET last_progress_at = now() WHERE run_id = $1")
            .bind(run_id)
            .execute(&self.pool)
            .await
            .context("updating run heartbeat")?;
        Ok(())
    }

    /// Mark a run as ended badly. `status` is `stalled` or `failed`; the
    /// reason is stored so the history says *why* a gap exists rather than
    /// only that one does.
    pub async fn fail_run(&self, run_id: Uuid, status: &str, reason: &str) -> Result<()> {
        sqlx::query(
            "UPDATE runs SET status = $2, failure_reason = $3, finished_at = now() \
             WHERE run_id = $1",
        )
        .bind(run_id)
        .bind(status)
        .bind(reason)
        .execute(&self.pool)
        .await
        .context("marking run failed")?;
        Ok(())
    }

    /// How many agents this run has written, straight from the rows. The
    /// watchdog compares this against itself over time; reading the table
    /// rather than an in-process counter means a sweep that is alive but
    /// writing nothing still counts as stalled.
    pub async fn swept_count(&self, run_id: Uuid) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT count(*) FROM agent_snapshots WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&self.pool)
            .await
            .context("counting swept agents")?;
        Ok(row.0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rung 6 (`live`) — the `liveness` binary's half of this store.
// ─────────────────────────────────────────────────────────────────────────────

/// One agent's rung-6 starting point: the document it published, and what rung
/// 4 made of it.
///
/// The document is re-parsed from `http_archive.body` rather than read from a
/// projected column, for the same reason rungs 3 and 4 parse it: the archived
/// bytes are the evidence, and anything derived from them is a second
/// implementation free to disagree with the first.
pub struct Rung6Candidate {
    pub agent_id: u64,
    /// `None` when the archive kept no body — nothing to read `services` from.
    pub body: Option<Vec<u8>>,
    /// Rung 4's status for this agent, verbatim from the row. Rung 6 depends
    /// on rung 4, and `checks::run_ladder` — never this crate — decides what a
    /// non-`pass` here means.
    pub rung4_status: String,
}

impl Db {
    /// The newest FINISHED run for a chain. What `liveness <chain>` resolves
    /// when given no explicit run id.
    ///
    /// Finished only: rung 6 reads a run's documents, and a sweep still
    /// writing them would give a different answer depending on when the probe
    /// happened to start.
    pub async fn latest_finished_run(&self, chain: &str) -> Result<Option<(Uuid, i32)>> {
        let row: Option<(Uuid, Option<i32>)> = sqlx::query_as(
            "SELECT run_id, schema_version FROM runs \
             WHERE chain = $1 AND finished_at IS NOT NULL AND status = 'finished' \
             ORDER BY started_at DESC LIMIT 1",
        )
        .bind(chain)
        .fetch_optional(&self.pool)
        .await
        .context("finding the latest finished run")?;
        Ok(row.map(|(id, v)| (id, v.unwrap_or(0))))
    }

    /// Every agent in a run, with its archived document body and rung 4's
    /// verdict.
    ///
    /// A LEFT JOIN on rung 4, not an inner one: an agent whose rung 4 row is
    /// missing entirely must still be visible here, so the caller can decide
    /// what that means rather than have it silently vanish from the
    /// population. `''` stands in for a missing status and is not a status any
    /// rung produces, so it cannot be mistaken for one.
    pub async fn rung6_candidates(&self, run_id: Uuid) -> Result<Vec<Rung6Candidate>> {
        let rows: Vec<(i64, Option<Vec<u8>>, Option<String>)> = sqlx::query_as(
            // `chain` is in both join conditions, taken from `a` rather than
            // passed in: every one of these tables is keyed
            // (run_id, chain, agent_id, …), so a join that omits it can only
            // use the leading column of the index. It costs nothing to add —
            // a run is one chain, so `a.chain` is a constant here — and it is
            // what lets the planner reach the rows by index instead of
            // hashing the whole run.
            "SELECT a.agent_id, h.body, c.status \
               FROM agent_snapshots a \
               LEFT JOIN http_archive h \
                 ON h.run_id = a.run_id AND h.chain = a.chain AND h.agent_id = a.agent_id \
               LEFT JOIN check_results c \
                 ON c.run_id = a.run_id AND c.chain = a.chain AND c.agent_id = a.agent_id \
                    AND c.rung = 4 \
              WHERE a.run_id = $1 \
              ORDER BY a.agent_id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .context("loading rung 6 candidates")?;
        Ok(rows
            .into_iter()
            .map(|(agent_id, body, rung4_status)| Rung6Candidate {
                agent_id: agent_id as u64,
                body,
                rung4_status: rung4_status.unwrap_or_default(),
            })
            .collect())
    }

    /// The URLs this run has already probed, with their observations.
    ///
    /// This IS the checkpoint. A probe pass that dies at hour two resumes by
    /// reading what landed rather than by trusting a file, and re-running it
    /// from scratch sends no request it has already sent.
    pub async fn probed_urls(&self, run_id: Uuid) -> Result<HashMap<String, ProbeRow>> {
        /// `(url, final_url, http_status, error, elapsed_ms)`, as the columns
        /// come back.
        type ProbeTuple = (
            String,
            Option<String>,
            Option<i32>,
            Option<String>,
            Option<i32>,
        );
        let rows: Vec<ProbeTuple> = sqlx::query_as(
            "SELECT url, final_url, http_status, error, elapsed_ms \
                   FROM endpoint_probes WHERE run_id = $1",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .context("loading already-probed URLs")?;
        Ok(rows
            .into_iter()
            .map(|(url, final_url, http_status, error, elapsed_ms)| {
                (
                    url,
                    ProbeRow {
                        final_url,
                        http_status: http_status.map(|s| s as u16),
                        error,
                        elapsed_ms: elapsed_ms.map(|ms| ms as u32),
                    },
                )
            })
            .collect())
    }

    /// Record one URL's observation.
    ///
    /// `ON CONFLICT DO NOTHING`, so a resumed pass that races itself keeps the
    /// FIRST observation rather than overwriting it. Two answers from the same
    /// URL in one run would make the run non-reproducible, and the earlier one
    /// is the one the checkpoint already promised.
    pub async fn record_probe(
        &self,
        run_id: Uuid,
        url: &str,
        host: &str,
        declared_by: i32,
        outcome: &probe::FetchOutcome,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO endpoint_probes \
               (run_id, url, host, declared_by, final_url, http_status, error, elapsed_ms) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8) \
             ON CONFLICT (run_id, url) DO NOTHING",
        )
        .bind(run_id)
        .bind(url)
        .bind(host)
        .bind(declared_by)
        .bind(&outcome.final_url)
        .bind(outcome.http_status.map(i32::from))
        .bind(&outcome.error)
        .bind(outcome.elapsed_ms.map(|ms| ms as i32))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Write one agent's rung-6 row, replacing any earlier one for the same
    /// run.
    ///
    /// Replacing rather than inserting because the probe pass is re-runnable
    /// against a run that already has rung-6 rows — a second pass with a wider
    /// budget should update the verdict, not double it. Every other rung is
    /// insert-only because the sweep that writes them runs once.
    pub async fn replace_rung6(
        &self,
        run_id: Uuid,
        chain: &str,
        agent_id: u64,
        result: &checks::CheckResult,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        // `chain` is in the predicate for the reason recorded on
        // `mark_rung2_refused`: `check_results_unique` is
        // (run_id, chain, agent_id, rung), so a predicate that skips `chain`
        // seeks on `run_id` alone and then walks every row the run wrote.
        // Measured on production against the 2026-08 BNB Chain run, one agent:
        // 1,549 ms without, 1.7 ms with. This runs ONCE PER AGENT — on that
        // run it is the difference between 36 hours and two minutes.
        sqlx::query(
            "DELETE FROM check_results \
             WHERE run_id = $1 AND chain = $3 AND agent_id = $2 AND rung = 6",
        )
        .bind(run_id)
        .bind(agent_id as i64)
        .bind(chain)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO check_results \
               (run_id, chain, agent_id, rung, name, status, evidence, checked_at) \
             VALUES ($1,$2,$3,6,$4,$5,$6,$7)",
        )
        .bind(run_id)
        .bind(chain)
        .bind(agent_id as i64)
        .bind(result.name)
        .bind(result.status.as_str())
        .bind(&result.evidence)
        .bind(result.checked_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Remove any rung-6 row for an agent this pass decided not to answer for.
    ///
    /// Needed because `replace_rung6` cannot express absence, and absence is a
    /// real outcome here — an agent whose every URL fell outside its host's
    /// budget gets no row. Without this, a re-run with a smaller budget would
    /// leave a stale verdict standing for an agent it did not probe.
    ///
    /// `chain` is a parameter for the same reason it is one on
    /// `replace_rung6`: without it in the predicate this walks every row the
    /// run wrote, once per agent. 1,549 ms against 1.7 ms on the 2026-08 BNB
    /// Chain run.
    pub async fn clear_rung6(
        &self,
        run_id: Uuid,
        chain: &str,
        agent_id: u64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "DELETE FROM check_results \
             WHERE run_id = $1 AND chain = $3 AND agent_id = $2 AND rung = 6",
        )
        .bind(run_id)
        .bind(agent_id as i64)
        .bind(chain)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Stamp the run as having been judged by this checker build.
    ///
    /// Rung 6 lands after `close_run` has already written the run's
    /// provenance, so without this a run would claim the `schema_version` and
    /// `checker_version` of the sweep that produced rungs 1-5/7 while
    /// containing rung-6 rows a later build wrote. A citation has to resolve
    /// to one set of semantics.
    pub async fn restamp_checker(
        &self,
        run_id: Uuid,
        schema_version: i32,
        checker_version: &str,
    ) -> Result<()> {
        sqlx::query("UPDATE runs SET schema_version = $2, checker_version = $3 WHERE run_id = $1")
            .bind(run_id)
            .bind(schema_version)
            .bind(checker_version)
            .execute(&self.pool)
            .await
            .context("restamping run provenance after the rung 6 pass")?;
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Re-judging an existing run — the `backfill-refused` binary's half.
// ─────────────────────────────────────────────────────────────────────────────
//
// Every method here writes to rows a run already published, which nothing else
// in this file is allowed to do. They exist for one reason and are worth
// keeping expensive to reach: a status whose DEFINITION changed leaves every
// earlier run saying something the vocabulary no longer means, and a series
// where 2026-07 says `fail` and 2026-08 says `refused` for the same 429 is not
// a series. See `bin/backfill-refused.rs` for the whole argument.

impl Db {
    /// Every run that has results, oldest first.
    pub async fn runs_with_results(&self) -> Result<Vec<(Uuid, String)>> {
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT r.run_id, r.chain FROM runs r \
             WHERE EXISTS (SELECT 1 FROM check_results c WHERE c.run_id = r.run_id) \
             ORDER BY r.started_at",
        )
        .fetch_all(&self.pool)
        .await
        .context("listing runs with results")?;
        Ok(rows)
    }

    /// One rung's status breakdown for one run — the before/after picture a
    /// reclassification has to report, read the same way `/api/runs/{id}/rates`
    /// reads it.
    pub async fn rung_status_counts(&self, run_id: Uuid, rung: i16) -> Result<Vec<(String, i64)>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT status, count(*) FROM check_results \
             WHERE run_id = $1 AND rung = $2 GROUP BY status ORDER BY status",
        )
        .bind(run_id)
        .bind(rung)
        .fetch_all(&self.pool)
        .await
        .context("counting rung statuses")?;
        Ok(rows)
    }

    /// Every rung-2 row of a run that could possibly move under the `refused`
    /// rules, with its evidence.
    ///
    /// Deliberately fetched and judged in Rust rather than matched in SQL: the
    /// predicate is `checks::refusal`, and a WHERE clause listing the status
    /// codes would be a second copy of it that no test compares against the
    /// first. `pass`, `skipped` and existing `refused` rows cannot move and are
    /// not read.
    pub async fn rung2_candidates(&self, run_id: Uuid) -> Result<Vec<(i64, String, JsonValue)>> {
        let rows: Vec<(i64, String, JsonValue)> = sqlx::query_as(
            "SELECT agent_id, status, evidence FROM check_results \
             WHERE run_id = $1 AND rung = 2 AND status IN ('fail', 'error')",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .context("reading rung-2 rows to re-judge")?;
        Ok(rows)
    }

    /// Move a batch of rung-2 rows to `refused`.
    ///
    /// `set_declined_reason` rewrites `evidence.reason` to `declined` for the
    /// rows whose old reason was the generic `http_status`, so a backfilled row
    /// and a freshly swept one are identical rather than merely equivalent. It
    /// is false for the rows whose reason already says something the new
    /// checker would also have written — `payment_required`, and every
    /// `robots_*` reason, which are carried through untouched.
    /// `chain` is redundant — a run has exactly one — and leaving it out made
    /// this unusably slow. `check_results_unique` is `(run_id, chain,
    /// agent_id, rung)`, so a predicate naming `run_id` and `agent_id` but not
    /// `chain` can seek on the first column only, then scans every row the run
    /// wrote to test the rest. Measured on the 2026-08 BNB Chain run, one
    /// 5,000-id chunk:
    ///
    ///   without chain   > 120 s — hit the statement timeout, never completed
    ///   with chain        74 ms — Index Scan using check_results_unique
    ///
    /// At least three orders of magnitude, for a column the caller is already
    /// holding. The same gap is why the homepage's findings had to be
    /// materialised in migration 0021: `chain` sits between `run_id` and
    /// `agent_id` in that key, so every per-agent lookup that does not spell
    /// the chain out falls off the index entirely.
    pub async fn mark_rung2_refused(
        &self,
        run_id: Uuid,
        chain: &str,
        agent_ids: &[i64],
        set_declined_reason: bool,
    ) -> Result<u64> {
        if agent_ids.is_empty() {
            return Ok(0);
        }
        let sql = if set_declined_reason {
            "UPDATE check_results \
                SET status = 'refused', \
                    evidence = jsonb_set(evidence, '{reason}', '\"declined\"') \
              WHERE run_id = $1 AND chain = $3 AND rung = 2 AND agent_id = ANY($2)"
        } else {
            "UPDATE check_results SET status = 'refused' \
              WHERE run_id = $1 AND chain = $3 AND rung = 2 AND agent_id = ANY($2)"
        };
        let done = sqlx::query(sql)
            .bind(run_id)
            .bind(agent_ids)
            .bind(chain)
            .execute(&self.pool)
            .await
            .context("reclassifying rung-2 rows as refused")?;
        Ok(done.rows_affected())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Week-over-week deltas — the `delta` binary's half of this store.
// ─────────────────────────────────────────────────────────────────────────────

/// Everything one `run_deltas` row needs, bundled so this method's signature
/// does not grow an argument every time a new figure is worth publishing.
pub struct DeltaWrite<'a> {
    pub run_id: Uuid,
    pub previous_run_id: Uuid,
    pub chain: &'a str,
    pub agents_before: i32,
    pub agents_after: i32,
    pub newly_registered: i32,
    pub disappeared: i32,
    pub newly_resolving: i32,
    pub stopped_resolving: i32,
    pub flips: &'a serde_json::Value,
    /// The two runs' checker builds. When they differ, some flips are method
    /// changes rather than changes in the world — see migration 0016.
    pub checker_before: &'a str,
    pub checker_after: &'a str,
    pub schema_before: i32,
    pub schema_after: i32,
}

impl Db {
    /// The most recent finished runs for a chain, newest first.
    pub async fn finished_runs(&self, chain: &str, limit: i64) -> Result<Vec<Uuid>> {
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT run_id FROM runs \
             WHERE chain = $1 AND finished_at IS NOT NULL AND status = 'finished' \
             ORDER BY started_at DESC LIMIT $2",
        )
        .bind(chain)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("listing finished runs")?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    /// Every `(agent_id, rung) -> status` this run recorded.
    ///
    /// The whole run in memory, deliberately. Comparing two 244,208-agent runs
    /// is a full join either way; doing it in Rust rather than SQL keeps the
    /// rule about what counts as a "flip" — both sides must have a row — in
    /// one readable place next to the reasoning for it, rather than encoded in
    /// a join condition where the next person reads it as an optimisation.
    pub async fn rung_statuses(&self, run_id: Uuid) -> Result<HashMap<(u64, i16), String>> {
        let rows: Vec<(i64, i16, String)> =
            sqlx::query_as("SELECT agent_id, rung, status FROM check_results WHERE run_id = $1")
                .bind(run_id)
                .fetch_all(&self.pool)
                .await
                .context("loading rung statuses")?;
        Ok(rows
            .into_iter()
            .map(|(agent, rung, status)| ((agent as u64, rung), status))
            .collect())
    }

    /// Write (or replace) a run's delta.
    ///
    /// Replaceable, unlike a run's own results: a delta is DERIVED, so
    /// recomputing it after fixing the computation is legitimate in a way that
    /// rewriting a measurement never is.
    pub async fn write_delta(&self, d: &DeltaWrite<'_>) -> Result<()> {
        sqlx::query(
            "INSERT INTO run_deltas \
               (run_id, previous_run_id, chain, agents_before, agents_after, \
                newly_registered, disappeared, newly_resolving, stopped_resolving, flips, \
                checker_before, checker_after, schema_before, schema_after) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14) \
             ON CONFLICT (run_id) DO UPDATE SET \
               previous_run_id = EXCLUDED.previous_run_id, \
               agents_before = EXCLUDED.agents_before, \
               agents_after = EXCLUDED.agents_after, \
               newly_registered = EXCLUDED.newly_registered, \
               disappeared = EXCLUDED.disappeared, \
               newly_resolving = EXCLUDED.newly_resolving, \
               stopped_resolving = EXCLUDED.stopped_resolving, \
               flips = EXCLUDED.flips, \
               checker_before = EXCLUDED.checker_before, \
               checker_after = EXCLUDED.checker_after, \
               schema_before = EXCLUDED.schema_before, \
               schema_after = EXCLUDED.schema_after, \
               computed_at = now()",
        )
        .bind(d.run_id)
        .bind(d.previous_run_id)
        .bind(d.chain)
        .bind(d.agents_before)
        .bind(d.agents_after)
        .bind(d.newly_registered)
        .bind(d.disappeared)
        .bind(d.newly_resolving)
        .bind(d.stopped_resolving)
        .bind(d.flips)
        .bind(d.checker_before)
        .bind(d.checker_after)
        .bind(d.schema_before)
        .bind(d.schema_after)
        .execute(&self.pool)
        .await
        .context("writing the run delta")?;
        Ok(())
    }

    /// Every delta row that exists, newest first.
    ///
    /// Only `backfill-refused` needs this: re-judging a run's results changes
    /// what its delta means, and a delta nobody recomputed would keep reporting
    /// churn that the reclassification just removed. Every delta is recomputed
    /// rather than only the ones whose `run_id` moved, because a delta reads
    /// BOTH runs and the older one may be the reclassified side.
    pub async fn all_deltas(&self) -> Result<Vec<(Uuid, Uuid, String)>> {
        let rows: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
            "SELECT run_id, previous_run_id, chain FROM run_deltas ORDER BY computed_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("listing deltas to recompute")?;
        Ok(rows)
    }

    /// Compute one run's findings and store them, replacing any earlier
    /// computation.
    ///
    /// The arithmetic is `ls_run_findings()` (migration 0021) and is NOT
    /// repeated here — the API's `/findings` endpoint calls the same function
    /// when a run has no stored row, and two implementations of a published
    /// figure is exactly the drift this project exists to refuse. This method
    /// is the database plumbing around it and nothing else.
    ///
    /// Recomputing is legitimate: a finding is derived from `check_results`,
    /// which this never writes. `computed_at` moves so a reader can tell a
    /// fresh derivation from the one published with the run.
    ///
    /// Returns how many findings were written. Zero means the run has no
    /// results — nothing is stored for it, because a row of zeroes reads as
    /// "we asked and found none".
    pub async fn write_findings(&self, run_id: Uuid) -> Result<u64> {
        let n = sqlx::query(
            "INSERT INTO run_findings (run_id, finding_key, numerator, denominator) \
             SELECT $1, f.finding_key, f.numerator, f.denominator \
             FROM ls_run_findings($1) f \
             WHERE EXISTS (SELECT 1 FROM check_results c WHERE c.run_id = $1) \
             ON CONFLICT (run_id, finding_key) DO UPDATE SET \
               numerator = EXCLUDED.numerator, \
               denominator = EXCLUDED.denominator, \
               computed_at = now()",
        )
        .bind(run_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("writing findings for run {run_id}"))?
        .rows_affected();
        Ok(n)
    }

    /// A run's stored findings, by key. Only the `findings` binary needs this,
    /// to print what it just wrote.
    pub async fn run_findings(&self, run_id: Uuid) -> Result<Vec<(String, i64, i64)>> {
        let rows = sqlx::query_as(
            "SELECT finding_key, numerator, denominator FROM run_findings \
             WHERE run_id = $1 ORDER BY finding_key",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .with_context(|| format!("reading findings for run {run_id}"))?;
        Ok(rows)
    }

    /// A run's checker build and schema version, for the delta's confound
    /// columns.
    pub async fn run_provenance(&self, run_id: Uuid) -> Result<(String, i32)> {
        let row: (String, i32) =
            sqlx::query_as("SELECT checker_version, schema_version FROM runs WHERE run_id = $1")
                .bind(run_id)
                .fetch_one(&self.pool)
                .await
                .with_context(|| format!("reading provenance for run {run_id}"))?;
        Ok(row)
    }
}

/// One archived probe observation, as read back for a resume.
#[derive(Debug, Clone)]
pub struct ProbeRow {
    pub final_url: Option<String>,
    pub http_status: Option<u16>,
    pub error: Option<String>,
    pub elapsed_ms: Option<u32>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Payments — the `payments` binary's half of this store (migration 0019).
// ─────────────────────────────────────────────────────────────────────────────
//
// Three tables, written by one pass: `payment_targets` (the attribution map),
// `payment_scans` (what was looked at) and `payments` (the transfers and their
// verdicts). Every one of them is `run_id`-scoped exactly like `check_results`,
// so a figure is pinned to a block and recomputable — which the one-off study
// these replace was not.

/// One agent as the payments pass needs it: who owns it, when it came into
/// existence, and what its document declared.
pub struct PaymentCandidate {
    pub agent_id: u64,
    /// `ownerOf` at the pinned block, from `agent_snapshots`.
    pub owner: String,
    /// `None` when this run predates minter capture (migration 0013) or the
    /// registration event could not be read. Fatal to attribution — see
    /// `payments::Exclusion::MintBlockUnknown` — and never silently treated as
    /// zero.
    pub registration_block: Option<u64>,
    /// The archived registration document, for the declared basis. `None` when
    /// the archive kept no body.
    pub body: Option<Vec<u8>>,
}

/// One row for `payment_targets`, assembled by the binary from
/// `payments::TargetDecision`.
pub struct TargetWrite<'a> {
    pub run_id: Uuid,
    pub chain: &'a str,
    pub agent_id: u64,
    pub basis: &'a str,
    pub address: &'a str,
    pub declared_index: Option<i32>,
    pub eligible: bool,
    pub ineligible_reason: Option<&'a str>,
    pub owner: &'a str,
    pub registration_block: Option<u64>,
    pub read_at_block: u64,
}

/// One row for `payments`.
pub struct PaymentWrite<'a> {
    pub run_id: Uuid,
    pub chain: &'a str,
    pub agent_id: u64,
    pub basis: &'a str,
    pub credited_address: &'a str,
    pub address_reached_by: i32,
    pub token_address: &'a str,
    pub token_symbol: &'a str,
    pub token_decimals: i16,
    pub direction: &'a str,
    pub counterparty: &'a str,
    /// The raw uint256 as a decimal string, cast to `NUMERIC` in the statement.
    /// Never narrowed to an integer type on the way through.
    pub value_raw: &'a str,
    pub block_number: u64,
    pub tx_hash: &'a str,
    pub log_index: i32,
    pub agent_registration_block: Option<u64>,
    pub post_mint: Option<bool>,
    pub counterparty_is_contract: Option<bool>,
    pub counterparty_is_run_owner: Option<bool>,
    pub eip3009_authorization: bool,
    pub eip3009_authorizer: Option<&'a str>,
    pub eip3009_authorizer_is_sender: Option<bool>,
    pub included: bool,
    pub exclusion: Option<&'a str>,
}

/// One row for `payment_scans`.
pub struct ScanWrite<'a> {
    pub run_id: Uuid,
    pub chain: &'a str,
    pub token_address: &'a str,
    pub token_symbol: &'a str,
    pub token_decimals: i16,
    pub from_block: u64,
    pub to_block: u64,
    pub directions: &'a str,
    pub basis: &'a str,
    pub targets_scanned: i32,
    pub transfers_found: i32,
    pub rule_version: &'a str,
}

impl Db {
    /// A finished run's chain and pinned block.
    ///
    /// Errors if the run has no `pinned_block`. There is no fallback: a
    /// payments pass with no pin would read logs up to whatever block the node
    /// had reached, and a figure assembled that way describes a population that
    /// never simultaneously existed — the exact failure pinning exists for.
    pub async fn run_pin(&self, run_id: Uuid) -> Result<(String, u64)> {
        let row: (String, Option<i64>) =
            sqlx::query_as("SELECT chain, pinned_block FROM runs WHERE run_id = $1")
                .bind(run_id)
                .fetch_one(&self.pool)
                .await
                .with_context(|| format!("no run {run_id}"))?;
        let pinned = row.1.with_context(|| {
            format!("run {run_id} has no pinned_block — nothing to pin a payments pass to")
        })?;
        Ok((row.0, pinned as u64))
    }

    /// Every agent in a run, with its owner, registration block and archived
    /// document.
    ///
    /// A LEFT JOIN on the archive, for the same reason `rung6_candidates` uses
    /// one: an agent with no archived body must still be visible, so the caller
    /// decides what that means rather than have it vanish from the population.
    pub async fn payment_candidates(&self, run_id: Uuid) -> Result<Vec<PaymentCandidate>> {
        /// `(agent_id, owner, registration_block, body)`, as the columns come
        /// back. Named for the same reason `ProbeTuple` above is.
        type CandidateTuple = (i64, String, Option<i64>, Option<Vec<u8>>);
        let rows: Vec<CandidateTuple> = sqlx::query_as(
            // `chain` in the join condition, taken from `a` rather than passed
            // in: `http_archive` is keyed (run_id, chain, agent_id), so
            // omitting it leaves the planner the leading column only. See
            // `scripts/check-chain-predicates.py` for why this keeps happening.
            "SELECT a.agent_id, a.owner, a.registration_block, h.body \
               FROM agent_snapshots a \
               LEFT JOIN http_archive h \
                 ON h.run_id = a.run_id AND h.chain = a.chain AND h.agent_id = a.agent_id \
              WHERE a.run_id = $1 \
              ORDER BY a.agent_id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .context("loading payment candidates")?;
        Ok(rows
            .into_iter()
            .map(
                |(agent_id, owner, registration_block, body)| PaymentCandidate {
                    agent_id: agent_id as u64,
                    owner,
                    registration_block: registration_block.map(|b| b as u64),
                    body,
                },
            )
            .collect())
    }

    /// Everything the payments pass wrote for a run, removed.
    ///
    /// The pass is re-runnable — a wider token list or a fixed rule should
    /// replace a run's payment rows, not double them — and a partial replace
    /// would leave rows judged under two different `rule_version`s in one run.
    /// All three tables are cleared in one transaction so a crash cannot leave
    /// scans without their transfers or targets without their map.
    pub async fn clear_payments(&self, run_id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for table in ["payments", "payment_scans", "payment_targets"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE run_id = $1"))
                .bind(run_id)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("clearing {table}"))?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn write_payment_target(&self, t: &TargetWrite<'_>) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO payment_targets \
               (run_id, chain, agent_id, basis, address, declared_index, eligible, \
                ineligible_reason, owner, registration_block, read_at_block) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) \
             ON CONFLICT (run_id, chain, agent_id, basis, address) DO NOTHING",
        )
        .bind(t.run_id)
        .bind(t.chain)
        .bind(t.agent_id as i64)
        .bind(t.basis)
        .bind(t.address)
        .bind(t.declared_index)
        .bind(t.eligible)
        .bind(t.ineligible_reason)
        .bind(t.owner)
        .bind(t.registration_block.map(|b| b as i64))
        .bind(t.read_at_block as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn write_payment(&self, p: &PaymentWrite<'_>) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO payments \
               (run_id, chain, agent_id, basis, credited_address, address_reached_by, \
                token_address, token_symbol, token_decimals, direction, counterparty, \
                value_raw, block_number, tx_hash, log_index, agent_registration_block, \
                post_mint, counterparty_is_contract, counterparty_is_run_owner, \
                eip3009_authorization, eip3009_authorizer, eip3009_authorizer_is_sender, \
                included, exclusion) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12::numeric,$13,$14,$15,$16,$17,\
                     $18,$19,$20,$21,$22,$23,$24) \
             ON CONFLICT ON CONSTRAINT payments_unique DO NOTHING",
        )
        .bind(p.run_id)
        .bind(p.chain)
        .bind(p.agent_id as i64)
        .bind(p.basis)
        .bind(p.credited_address)
        .bind(p.address_reached_by)
        .bind(p.token_address)
        .bind(p.token_symbol)
        .bind(p.token_decimals)
        .bind(p.direction)
        .bind(p.counterparty)
        .bind(p.value_raw)
        .bind(p.block_number as i64)
        .bind(p.tx_hash)
        .bind(p.log_index)
        .bind(p.agent_registration_block.map(|b| b as i64))
        .bind(p.post_mint)
        .bind(p.counterparty_is_contract)
        .bind(p.counterparty_is_run_owner)
        .bind(p.eip3009_authorization)
        .bind(p.eip3009_authorizer)
        .bind(p.eip3009_authorizer_is_sender)
        .bind(p.included)
        .bind(p.exclusion)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Record that a (basis, token) was scanned. Written LAST, after the
    /// transfers it describes, so a crash mid-scan leaves no row claiming a
    /// range was covered when it was not — absence means "not scanned", and
    /// that has to stay true under failure, not only under success.
    pub async fn write_payment_scan(&self, s: &ScanWrite<'_>) -> Result<()> {
        sqlx::query(
            "INSERT INTO payment_scans \
               (run_id, chain, token_address, token_symbol, token_decimals, from_block, \
                to_block, directions, basis, targets_scanned, transfers_found, rule_version) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) \
             ON CONFLICT (run_id, basis, token_address) DO UPDATE SET \
               token_symbol = EXCLUDED.token_symbol, \
               token_decimals = EXCLUDED.token_decimals, \
               from_block = EXCLUDED.from_block, \
               to_block = EXCLUDED.to_block, \
               directions = EXCLUDED.directions, \
               targets_scanned = EXCLUDED.targets_scanned, \
               transfers_found = EXCLUDED.transfers_found, \
               rule_version = EXCLUDED.rule_version, \
               scanned_at = now()",
        )
        .bind(s.run_id)
        .bind(s.chain)
        .bind(s.token_address)
        .bind(s.token_symbol)
        .bind(s.token_decimals)
        .bind(s.from_block as i64)
        .bind(s.to_block as i64)
        .bind(s.directions)
        .bind(s.basis)
        .bind(s.targets_scanned)
        .bind(s.transfers_found)
        .bind(s.rule_version)
        .execute(&self.pool)
        .await
        .context("recording a payment scan")?;
        Ok(())
    }
}
