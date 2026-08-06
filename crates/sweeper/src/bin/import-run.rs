//! # import-run — load a published run archive into a local Postgres.
//!
//! ```text
//! DATABASE_URL=… import-run [--replace] <path-to-archive.tar.zst | path-to-extracted-dir>
//! ```
//!
//! The inverse of `export-run`. DATA.md publishes every census run as a
//! `tar.zst` archive, but until this binary existed nothing could put one
//! INTO a database — so a contributor could not run `cargo run -p api`
//! against real data without sweeping a live chain themselves. This closes
//! that gap: download the smallest archive, import it, start the API.
//!
//! ## What it writes
//!
//! One `runs` row, one `agent_snapshots` row per agent file, one
//! `check_results` row per rung answered, and one `http_archive` row per
//! agent whose export carries any fetch summary. Three honesty notes:
//!
//! - **`http_archive.body` is never in an archive** (DATA.md: bodies can be
//!   1 MiB each and live only in the original database), so imported archive
//!   rows carry the summary columns only — status, content-type, size, hash,
//!   final URL. Re-judging rungs 3–5 from bodies is not possible on an
//!   imported run; re-reading the recorded verdicts and evidence is the point.
//! - An agent whose fetch summary is entirely absent from its file gets **no**
//!   `http_archive` row, because "every field was NULL" and "no row existed"
//!   export identically. Absence means "not recoverable from the archive".
//! - `http_archive.scheme` is NOT NULL but is not in the export as a column;
//!   it is recovered from rung 2's evidence (which records the same bucket),
//!   falling back to classifying the URI prefix.
//!
//! ## Idempotence
//!
//! A run that already exists is refused with its row counts, unless
//! `--replace` is passed — in which case the existing run's rows are deleted
//! and the import re-done, all inside one transaction: an interrupted import
//! leaves either the old run or the new one, never a mixture.
//!
//! ## Both manifest shapes
//!
//! A sweep-time manifest (`export.rs::RunManifest`) and a rebuilt one
//! (`export-run.rs::RebuiltManifest`, with `rebuilt_at` and possibly exporter
//! provenance fields) differ in which fields exist and which are null. Every
//! field either shape may omit is optional here, and unknown fields are
//! ignored, so archives produced by future exporters still import.
//!
//! ## Decompression
//!
//! Extraction shells out: `tar --zstd -xf` first, then `zstd -dc | tar -x`
//! as a fallback. One of `tar` with zstd support or the `zstd` binary must be
//! installed (`brew install zstd` / `apt install zstd`). Deliberately not an
//! in-process decoder: no zstd crate is in the dependency tree, and this tool
//! does not justify adding one. Passing an already-extracted directory skips
//! the requirement entirely.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sweeper::store;
use uuid::Uuid;

/// Union of the sweep-time and rebuilt manifest shapes. Everything that
/// either shape can omit or null is `Option`; unknown fields (e.g. the
/// exporter provenance a newer branch adds) are ignored by serde's default.
#[derive(Deserialize)]
struct Manifest {
    run_id: String,
    chain: String,
    #[serde(default)]
    chain_id: Option<i64>,
    /// The identity registry address. Used only to seed a minimal `chains`
    /// row when the chain is not configured locally.
    #[serde(default)]
    registry: Option<String>,
    #[serde(default)]
    pinned_block: Option<i64>,
    started_at: String,
    schema_version: i32,
    checker_version: String,
    checker_commit: String,
    spec_commit: String,
    rerun_command: String,
    #[serde(default)]
    agent_count: Option<i64>,
    /// How many agents were read and persisted — which is exactly what the
    /// archive contains and what `runs.agent_count` records at close.
    #[serde(default)]
    swept: Option<i64>,
    #[serde(default)]
    finished_at: Option<String>,
}

/// One `<chain>/<agent_id>.json`, both export flavours. `token_id` stays a
/// STRING end to end: it is a uint256, exceeds i64, and is bound to the
/// NUMERIC column via `::numeric` on the text value — same rule the sweeper
/// itself follows.
#[derive(Deserialize)]
struct AgentFile {
    agent_id: i64,
    token_id: String,
    owner: String,
    agent_uri: String,
    block_number: i64,
    #[serde(default)]
    minter: Option<String>,
    #[serde(default)]
    registration_tx_hash: Option<String>,
    #[serde(default)]
    registration_block: Option<i64>,
    #[serde(default)]
    checks: Vec<CheckEntry>,
    #[serde(default)]
    http_status: Option<i32>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    body_bytes: Option<i32>,
    #[serde(default)]
    body_sha256: Option<String>,
    #[serde(default)]
    final_url: Option<String>,
}

#[derive(Deserialize)]
struct CheckEntry {
    rung: i16,
    name: String,
    status: String,
    evidence: serde_json::Value,
    checked_at: String,
}

/// Agents per batched INSERT. 500 agents is ~500 snapshot rows and ~3,500
/// check rows per round trip — large enough that the biggest published run
/// (244,208 agents) imports in hundreds of statements rather than a million.
const BATCH: usize = 500;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut replace = false;
    let mut path: Option<std::path::PathBuf> = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--replace" => replace = true,
            _ if path.is_none() => path = Some(arg.into()),
            other => bail!("unexpected argument {other:?}"),
        }
    }
    let path = path.context("usage: import-run [--replace] <archive.tar.zst | extracted-dir>")?;
    if !path.exists() {
        bail!("{} does not exist", path.display());
    }

    // Extract if given an archive; scratch dir is removed on success.
    let (run_dir, scratch) = if path.is_dir() {
        (find_run_dir(&path)?, None)
    } else {
        let scratch = std::env::temp_dir().join(format!(
            "import-run-{}-{}",
            std::process::id(),
            Utc::now().timestamp()
        ));
        std::fs::create_dir_all(&scratch).context("creating extraction dir")?;
        extract(&path, &scratch)?;
        (find_run_dir(&scratch)?, Some(scratch))
    };

    let manifest_path = run_dir.join("manifest.json");
    let manifest: Manifest = serde_json::from_slice(
        &std::fs::read(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?,
    )
    .context("parsing manifest.json")?;
    let run_id: Uuid = manifest
        .run_id
        .parse()
        .context("manifest run_id is not a uuid")?;
    let chain = manifest.chain.clone();
    let started_at = parse_ts(&manifest.started_at).context("manifest started_at")?;
    let finished_at = manifest
        .finished_at
        .as_deref()
        .map(parse_ts)
        .transpose()
        .context("manifest finished_at")?;

    // Load every agent file up front: a file that does not parse should
    // refuse the import before a single row is written, not halfway through.
    let agents = load_agents(&run_dir.join(&chain))?;
    tracing::info!(
        "importing run {run_id} ({chain}, schema {}): {} agent files",
        manifest.schema_version,
        agents.len()
    );

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = store::Db::connect(&database_url).await?;

    ensure_chain(&db, &manifest).await?;

    let already: Option<(String,)> = sqlx::query_as("SELECT chain FROM runs WHERE run_id = $1")
        .bind(run_id)
        .fetch_optional(&db.pool)
        .await
        .context("checking for an existing run")?;
    if already.is_some() && !replace {
        bail!(
            "run {run_id} already exists in this database — pass --replace to delete it \
             and re-import"
        );
    }

    let mut tx = db.pool.begin().await.context("opening transaction")?;

    if already.is_some() {
        tracing::info!("--replace: deleting existing rows for run {run_id}");
        // Children without ON DELETE CASCADE first, then the run row —
        // endpoint_probes, run_deltas and run_findings cascade from it.
        for table in [
            "check_results",
            "http_archive",
            "agent_documents",
            "agent_snapshots",
        ] {
            sqlx::query(&format!("DELETE FROM {table} WHERE run_id = $1"))
                .bind(run_id)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("deleting from {table}"))?;
        }
        sqlx::query("DELETE FROM runs WHERE run_id = $1")
            .bind(run_id)
            .execute(&mut *tx)
            .await
            .context("deleting the runs row")?;
    }

    // `runs.agent_count` records how many were swept (see `close_run`), which
    // is what `swept` says and what the rebuilt manifest's `agent_count`
    // echoes back. The file count is the fallback for a manifest carrying
    // neither (an interrupted sweep's).
    let agent_count = manifest
        .swept
        .or(manifest.agent_count)
        .unwrap_or(agents.len() as i64) as i32;
    // The archive cannot say more than "finished or not": a run exported
    // without `finished_at` was interrupted, and 'unknown' is the status the
    // schema defines for "cannot say" (migration 0014).
    let status = if finished_at.is_some() {
        "finished"
    } else {
        "unknown"
    };
    sqlx::query(
        "INSERT INTO runs (run_id, chain, started_at, finished_at, schema_version, \
                           checker_version, checker_commit, spec_commit, rerun_command, \
                           agent_count, pinned_block, status) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
    )
    .bind(run_id)
    .bind(&chain)
    .bind(started_at)
    .bind(finished_at)
    .bind(manifest.schema_version)
    .bind(&manifest.checker_version)
    .bind(&manifest.checker_commit)
    .bind(&manifest.spec_commit)
    .bind(&manifest.rerun_command)
    .bind(agent_count)
    .bind(manifest.pinned_block)
    .bind(status)
    .execute(&mut *tx)
    .await
    .context("inserting the runs row")?;

    let mut snapshots = 0u64;
    let mut checks = 0u64;
    let mut archives = 0u64;
    for batch in agents.chunks(BATCH) {
        snapshots += insert_snapshots(&mut tx, run_id, &chain, batch).await?;
        checks += insert_checks(&mut tx, run_id, &chain, batch).await?;
        archives += insert_archives(&mut tx, run_id, &chain, batch).await?;
    }

    tx.commit().await.context("committing the import")?;

    if let Some(scratch) = scratch {
        let _ = std::fs::remove_dir_all(scratch);
    }

    // The homepage's figures, derived from the rows just written — the same
    // `ls_run_findings()` the sweep and the API use, so an imported run and a
    // swept one publish identical numbers. Done here rather than left to the
    // API's fallback because that fallback is the full count: on the BNB Chain
    // export (244,208 agents) it is the request that used to time out, and a
    // local API should not inherit the bug this import exists to help debug.
    let findings = db.write_findings(run_id).await?;

    println!(
        "imported run {run_id} ({chain}, schema {})",
        manifest.schema_version
    );
    println!("  runs             1");
    println!("  agent_snapshots  {snapshots}");
    println!("  check_results    {checks}");
    println!(
        "  http_archive     {archives}   (summary columns only — bodies are never in archives)"
    );
    println!("  run_findings     {findings}   (counted once here, so /findings need not count)");
    println!();
    println!("next: DATABASE_URL=… cargo run -p api   # then GET /api/runs");
    Ok(())
}

/// Extract `archive` into `dest`, preferring tar's built-in zstd support and
/// falling back to a `zstd -dc | tar -x` pipeline.
fn extract(archive: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    tracing::info!("extracting {} …", archive.display());
    let direct = std::process::Command::new("tar")
        .args(["--zstd", "-xf"])
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .output();
    if let Ok(out) = &direct
        && out.status.success()
    {
        return Ok(());
    }
    let mut zstd = std::process::Command::new("zstd")
        .args(["-dc", "--"])
        .arg(archive)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .context(
            "neither `tar --zstd` nor `zstd` worked — install zstd (brew install zstd / \
             apt install zstd), or extract the archive yourself and pass the directory",
        )?;
    let status = std::process::Command::new("tar")
        .args(["-xf", "-", "-C"])
        .arg(dest)
        .stdin(zstd.stdout.take().context("zstd stdout")?)
        .status()
        .context("running tar")?;
    let zstd_status = zstd.wait().context("waiting for zstd")?;
    if !status.success() || !zstd_status.success() {
        bail!(
            "extracting {} failed — is it a tar.zst archive? (tar exit {status}, zstd exit {zstd_status})",
            archive.display()
        );
    }
    Ok(())
}

/// The directory holding `manifest.json`: either `path` itself or the single
/// `<run_id>/` directory an archive expands to.
fn find_run_dir(path: &std::path::Path) -> Result<std::path::PathBuf> {
    if path.join("manifest.json").is_file() {
        return Ok(path.to_path_buf());
    }
    let mut candidates: Vec<_> = std::fs::read_dir(path)
        .with_context(|| format!("reading {}", path.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("manifest.json").is_file())
        .collect();
    match (candidates.pop(), candidates.is_empty()) {
        (Some(dir), true) => Ok(dir),
        (Some(_), false) => bail!(
            "{} holds more than one run directory — pass the specific <run_id>/ directory",
            path.display()
        ),
        (None, _) => bail!("no manifest.json under {}", path.display()),
    }
}

fn load_agents(chain_dir: &std::path::Path) -> Result<Vec<AgentFile>> {
    let mut agents = Vec::new();
    for entry in std::fs::read_dir(chain_dir).with_context(|| {
        format!(
            "reading {} — does the archive's chain directory exist?",
            chain_dir.display()
        )
    })? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let agent: AgentFile = serde_json::from_slice(
            &std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
        )
        .with_context(|| format!("parsing {}", path.display()))?;
        agents.push(agent);
    }
    if agents.is_empty() {
        bail!("no agent files in {}", chain_dir.display());
    }
    agents.sort_by_key(|a| a.agent_id);
    Ok(agents)
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(s)
        .with_context(|| format!("not an RFC 3339 timestamp: {s:?}"))?
        .with_timezone(&Utc))
}

/// Recursively apply the sweeper's NUL convention to every string in a JSON
/// tree: a raw NUL becomes the six literal characters `\` `u` `0` `0` `0` `0`,
/// which is what `store::escape_nuls_for_postgres` writes for TEXT columns.
/// Keys are escaped too — a NUL is illegal in a jsonb key just as in a value.
fn escape_nuls_in_json(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::String(s) => {
            if s.contains('\0') {
                *s = s.replace('\0', "\\u0000");
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(escape_nuls_in_json),
        serde_json::Value::Object(map) => {
            if map.keys().any(|k| k.contains('\0')) {
                let cleaned: serde_json::Map<String, serde_json::Value> = std::mem::take(map)
                    .into_iter()
                    .map(|(k, val)| (k.replace('\0', "\\u0000"), val))
                    .collect();
                *map = cleaned;
            }
            map.values_mut().for_each(escape_nuls_in_json);
        }
        _ => {}
    }
}

/// `runs.chain` has a foreign key into `chains`, so the chain must exist
/// before the run can. A missing chain gets a minimal row from the
/// manifest's own `chain_id` and `registry` — enough for the API and for
/// re-derivation, with `deploy_block 0` and `enabled false` because an
/// imported chain is data to read, not a chain this database is sweeping.
/// `scripts/seed_chains.sql` remains the real configuration and can be run
/// at any time; its upsert overwrites this stub.
async fn ensure_chain(db: &store::Db, m: &Manifest) -> Result<()> {
    let known: Option<(i64,)> = sqlx::query_as("SELECT chain_id FROM chains WHERE chain = $1")
        .bind(&m.chain)
        .fetch_optional(&db.pool)
        .await
        .context("checking the chains table")?;
    if let Some((chain_id,)) = known {
        if let Some(expected) = m.chain_id
            && chain_id != expected
        {
            bail!(
                "chains.{} has chain_id {chain_id} but the manifest says {expected} — \
                 refusing to mix two networks under one name",
                m.chain
            );
        }
        return Ok(());
    }
    let (Some(chain_id), Some(registry)) = (m.chain_id, m.registry.as_deref()) else {
        bail!(
            "chain {:?} is not in the chains table and this manifest does not carry \
             chain_id/registry to seed it — run scripts/seed_chains.sql first:\n  \
             psql \"$DATABASE_URL\" -f scripts/seed_chains.sql",
            m.chain
        );
    };
    sqlx::query(
        "INSERT INTO chains (chain, chain_id, identity_registry, deploy_block, enabled) \
         VALUES ($1, $2, $3, 0, false)",
    )
    .bind(&m.chain)
    .bind(chain_id)
    .bind(registry.to_lowercase())
    .execute(&db.pool)
    .await
    .context("seeding a minimal chains row")?;
    tracing::info!(
        "chains table had no {:?} — seeded a minimal disabled row from the manifest \
         (run scripts/seed_chains.sql for the full configuration)",
        m.chain
    );
    Ok(())
}

async fn insert_snapshots(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    run_id: Uuid,
    chain: &str,
    batch: &[AgentFile],
) -> Result<u64> {
    let mut ids = Vec::with_capacity(batch.len());
    let mut tokens = Vec::with_capacity(batch.len());
    let mut owners = Vec::with_capacity(batch.len());
    let mut uris = Vec::with_capacity(batch.len());
    let mut blocks = Vec::with_capacity(batch.len());
    let mut minters = Vec::with_capacity(batch.len());
    let mut reg_txs = Vec::with_capacity(batch.len());
    let mut reg_blocks = Vec::with_capacity(batch.len());
    for a in batch {
        ids.push(a.agent_id);
        tokens.push(a.token_id.clone());
        owners.push(a.owner.clone());
        // A sweep-time export can carry a raw NUL in tokenURI()'s value;
        // escape it the same lossless way the sweeper does before TEXT.
        uris.push(store::escape_nuls_for_postgres(a.agent_id as u64, &a.agent_uri).into_owned());
        blocks.push(a.block_number);
        minters.push(a.minter.clone());
        reg_txs.push(a.registration_tx_hash.clone());
        reg_blocks.push(a.registration_block);
    }
    let done = sqlx::query(
        "INSERT INTO agent_snapshots \
           (run_id, chain, agent_id, token_id, owner, agent_uri, block_number, \
            minter, registration_tx_hash, registration_block) \
         SELECT $1, $2, x.id, x.token::numeric, x.owner, x.uri, x.block, x.minter, x.tx, x.rb \
           FROM UNNEST($3::bigint[], $4::text[], $5::text[], $6::text[], $7::bigint[], \
                       $8::text[], $9::text[], $10::bigint[]) \
                AS x(id, token, owner, uri, block, minter, tx, rb)",
    )
    .bind(run_id)
    .bind(chain)
    .bind(&ids)
    .bind(&tokens)
    .bind(&owners)
    .bind(&uris)
    .bind(&blocks)
    .bind(&minters)
    .bind(&reg_txs)
    .bind(&reg_blocks)
    .execute(&mut **tx)
    .await
    .context("inserting agent_snapshots")?;
    Ok(done.rows_affected())
}

async fn insert_checks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    run_id: Uuid,
    chain: &str,
    batch: &[AgentFile],
) -> Result<u64> {
    let mut ids = Vec::new();
    let mut rungs = Vec::new();
    let mut names = Vec::new();
    let mut statuses = Vec::new();
    let mut evidences = Vec::new();
    let mut checked_ats = Vec::new();
    for a in batch {
        for c in &a.checks {
            ids.push(a.agent_id);
            rungs.push(c.rung);
            names.push(c.name.clone());
            statuses.push(c.status.clone());
            // Evidence is bound as JSON text and cast to jsonb server-side.
            // Postgres jsonb rejects a NUL anywhere in a value, so re-apply
            // the sweeper's convention first: a raw NUL inside any string in
            // the evidence tree becomes the six literal characters "\\u0000"
            // (see `store::escape_nuls_for_postgres`) -- lossless, grep-able,
            // never a silent drop. Done on the parsed tree, not the serialised
            // text, so a string that legitimately contains those characters is
            // never touched.
            let mut evidence = c.evidence.clone();
            escape_nuls_in_json(&mut evidence);
            evidences.push(serde_json::to_string(&evidence)?);
            checked_ats
                .push(parse_ts(&c.checked_at).with_context(|| {
                    format!("agent {}: rung {} checked_at", a.agent_id, c.rung)
                })?);
        }
    }
    if ids.is_empty() {
        return Ok(0);
    }
    let done = sqlx::query(
        "INSERT INTO check_results \
           (run_id, chain, agent_id, rung, name, status, evidence, checked_at) \
         SELECT $1, $2, x.id, x.rung, x.name, x.status, x.evidence::jsonb, x.at \
           FROM UNNEST($3::bigint[], $4::smallint[], $5::text[], $6::text[], $7::text[], \
                       $8::timestamptz[]) \
                AS x(id, rung, name, status, evidence, at)",
    )
    .bind(run_id)
    .bind(chain)
    .bind(&ids)
    .bind(&rungs)
    .bind(&names)
    .bind(&statuses)
    .bind(&evidences)
    .bind(&checked_ats)
    .execute(&mut **tx)
    .await
    .context("inserting check_results")?;
    Ok(done.rows_affected())
}

/// The scheme bucket `http_archive.scheme` requires. Rung 2's evidence
/// records the sweep's own classification; the URI prefix is the fallback
/// for an agent whose rung 2 never ran.
fn scheme_for(a: &AgentFile) -> String {
    if let Some(s) = a
        .checks
        .iter()
        .find(|c| c.rung == 2)
        .and_then(|c| c.evidence.get("scheme"))
        .and_then(|v| v.as_str())
    {
        return s.to_string();
    }
    let uri = a.agent_uri.trim();
    let lower = uri.to_ascii_lowercase();
    if uri.is_empty() {
        "empty"
    } else if lower.starts_with("https://") {
        "https"
    } else if lower.starts_with("http://") {
        "http"
    } else if lower.starts_with("ipfs://") {
        "ipfs"
    } else if lower.starts_with("data:") {
        "data"
    } else {
        "unsupported"
    }
    .to_string()
}

async fn insert_archives(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    run_id: Uuid,
    chain: &str,
    batch: &[AgentFile],
) -> Result<u64> {
    let mut ids = Vec::new();
    let mut uris = Vec::new();
    let mut schemes = Vec::new();
    let mut statuses = Vec::new();
    let mut content_types = Vec::new();
    let mut body_bytes = Vec::new();
    let mut body_shas = Vec::new();
    let mut final_urls = Vec::new();
    for a in batch {
        // A file carrying no fetch summary at all is indistinguishable from a
        // run that wrote no archive row — so write none. See the module doc.
        if a.http_status.is_none()
            && a.content_type.is_none()
            && a.body_bytes.is_none()
            && a.body_sha256.is_none()
            && a.final_url.is_none()
        {
            continue;
        }
        ids.push(a.agent_id);
        uris.push(store::escape_nuls_for_postgres(a.agent_id as u64, &a.agent_uri).into_owned());
        schemes.push(scheme_for(a));
        statuses.push(a.http_status);
        content_types.push(a.content_type.clone());
        body_bytes.push(a.body_bytes);
        body_shas.push(a.body_sha256.clone());
        final_urls.push(a.final_url.clone());
    }
    if ids.is_empty() {
        return Ok(0);
    }
    let done = sqlx::query(
        "INSERT INTO http_archive \
           (run_id, chain, agent_id, requested_uri, scheme, http_status, content_type, \
            body_bytes, body_sha256, final_url) \
         SELECT $1, $2, x.id, x.uri, x.scheme, x.status, x.ct, x.bytes, x.sha, x.final \
           FROM UNNEST($3::bigint[], $4::text[], $5::text[], $6::int[], $7::text[], \
                       $8::int[], $9::text[], $10::text[]) \
                AS x(id, uri, scheme, status, ct, bytes, sha, final)",
    )
    .bind(run_id)
    .bind(chain)
    .bind(&ids)
    .bind(&uris)
    .bind(&schemes)
    .bind(&statuses)
    .bind(&content_types)
    .bind(&body_bytes)
    .bind(&body_shas)
    .bind(&final_urls)
    .execute(&mut **tx)
    .await
    .context("inserting http_archive summaries")?;
    Ok(done.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sweep-time manifest (`export.rs::RunManifest`): no `rebuilt_at`, no
    /// exporter fields, counts as plain numbers.
    #[test]
    fn parses_sweep_time_manifest() {
        let m: Manifest = serde_json::from_str(
            r#"{
                "run_id": "7833fc49-a5b7-477b-99ce-946f650f0064",
                "chain": "celo", "chain_id": 42220,
                "registry": "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432",
                "pinned_block": 12345,
                "started_at": "2026-07-29T10:00:00Z",
                "schema_version": 6,
                "checker_version": "0.2.0",
                "checker_commit": "abc", "spec_commit": "def",
                "rerun_command": "cargo run --release -p sweeper -- celo",
                "agent_count": 9747, "swept": 9747,
                "unreadable": 0, "unwritable": 0,
                "finished_at": "2026-07-29T12:00:00Z"
            }"#,
        )
        .unwrap();
        assert_eq!(m.swept, Some(9747));
        assert_eq!(m.chain_id, Some(42220));
        assert!(m.finished_at.is_some());
    }

    /// A rebuilt manifest: `unreadable`/`unwritable` null, `rebuilt_at` set,
    /// plus exporter provenance fields a newer branch adds — all of which
    /// must be tolerated, none of which may be required.
    #[test]
    fn parses_rebuilt_manifest_with_exporter_fields() {
        let m: Manifest = serde_json::from_str(
            r#"{
                "run_id": "7833fc49-a5b7-477b-99ce-946f650f0064",
                "chain": "celo", "chain_id": 42220,
                "registry": "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432",
                "pinned_block": null,
                "started_at": "2026-07-29T10:00:00+00:00",
                "schema_version": 6,
                "checker_version": "0.2.0",
                "checker_commit": "abc", "spec_commit": "def",
                "rerun_command": "…",
                "agent_count": 9747, "swept": 9747,
                "unreadable": null, "unwritable": null,
                "finished_at": "2026-07-29T12:00:00+00:00",
                "rebuilt_at": "2026-08-01T00:00:00+00:00",
                "exporter_commit": "0123abc", "exporter_version": "0.2.1"
            }"#,
        )
        .unwrap();
        assert_eq!(m.pinned_block, None);
        assert_eq!(m.agent_count, Some(9747));
    }

    /// An interrupted sweep's manifest: written up front, `swept` and
    /// `finished_at` still null. Must parse — the import marks it 'unknown'.
    #[test]
    fn parses_interrupted_manifest() {
        let m: Manifest = serde_json::from_str(
            r#"{
                "run_id": "7833fc49-a5b7-477b-99ce-946f650f0064",
                "chain": "celo", "chain_id": 42220,
                "registry": "0x8004…", "pinned_block": 1,
                "started_at": "2026-07-29T10:00:00Z",
                "schema_version": 6, "checker_version": "0.2.0",
                "checker_commit": "abc", "spec_commit": "def",
                "rerun_command": "…", "agent_count": 9747,
                "swept": null, "unreadable": null, "unwritable": null,
                "finished_at": null
            }"#,
        )
        .unwrap();
        assert_eq!(m.swept, None);
        assert!(m.finished_at.is_none());
    }

    #[test]
    fn parses_agent_file_and_token_id_stays_text() {
        // token_id larger than i64::MAX — the reason the column is NUMERIC
        // and the reason this struct never touches an integer type for it.
        let a: AgentFile = serde_json::from_str(
            r#"{
                "run_id": "7833fc49-a5b7-477b-99ce-946f650f0064",
                "chain": "celo", "agent_id": 42,
                "token_id": "115792089237316195423570985008687907853269984665640564039457584007913129639935",
                "owner": "0xabc", "agent_uri": "https://example.com/agent.json",
                "block_number": 100,
                "minter": "0xdef", "registration_tx_hash": "0x123",
                "registration_block": 99,
                "checks": [
                    {"rung": 1, "name": "registered", "status": "pass",
                     "evidence": {"token_id": "1"}, "checked_at": "2026-07-29T10:00:00Z"},
                    {"rung": 2, "name": "resolvable", "status": "pass",
                     "evidence": {"scheme": "https"}, "checked_at": "2026-07-29T10:00:01Z"}
                ],
                "checker_commit": "abc", "spec_commit": "def",
                "http_status": 200, "content_type": "application/json",
                "body_bytes": 512, "body_sha256": "deadbeef",
                "final_url": "https://example.com/agent.json"
            }"#,
        )
        .unwrap();
        assert_eq!(a.token_id.len(), 78);
        assert_eq!(a.checks.len(), 2);
        assert_eq!(scheme_for(&a), "https"); // from rung 2 evidence
    }

    /// Pre-minter-capture archives (schema < 6) have no minter columns and no
    /// registration provenance; those fields must default rather than fail.
    #[test]
    fn parses_agent_file_without_schema6_fields() {
        let a: AgentFile = serde_json::from_str(
            r#"{
                "run_id": "x", "chain": "base", "agent_id": 7,
                "token_id": "7", "owner": "0xabc", "agent_uri": "",
                "block_number": 100, "checks": [],
                "checker_commit": "abc", "spec_commit": "def",
                "http_status": null, "content_type": null,
                "body_bytes": null, "body_sha256": null, "final_url": null
            }"#,
        )
        .unwrap();
        assert_eq!(a.minter, None);
        // No rung 2 row and an empty URI: the fallback classifier answers.
        assert_eq!(scheme_for(&a), "empty");
    }

    #[test]
    fn scheme_fallback_classifies_uri_prefixes() {
        let mk = |uri: &str| AgentFile {
            agent_id: 1,
            token_id: "1".into(),
            owner: "0x".into(),
            agent_uri: uri.into(),
            block_number: 1,
            minter: None,
            registration_tx_hash: None,
            registration_block: None,
            checks: vec![],
            http_status: None,
            content_type: None,
            body_bytes: None,
            body_sha256: None,
            final_url: None,
        };
        assert_eq!(scheme_for(&mk("https://a.example/x")), "https");
        assert_eq!(scheme_for(&mk("HTTP://a.example/x")), "http");
        assert_eq!(scheme_for(&mk("ipfs://Qm…")), "ipfs");
        assert_eq!(scheme_for(&mk("data:application/json;base64,e30=")), "data");
        assert_eq!(scheme_for(&mk("")), "empty");
        assert_eq!(scheme_for(&mk("mailto:a@example.com")), "unsupported");
    }

    #[test]
    fn nul_escaping_walks_the_whole_tree() {
        let mut v = serde_json::json!({
            "clean": "fine",
            "dirty": "a\u{0}b",
            "nested": [{"k": "x\u{0}"}],
        });
        escape_nuls_in_json(&mut v);
        assert_eq!(v["dirty"], "a\\u0000b");
        assert_eq!(v["nested"][0]["k"], "x\\u0000");
        assert_eq!(v["clean"], "fine");
        // The result round-trips through JSON text without any raw NUL —
        // which is the property jsonb needs.
        assert!(!serde_json::to_string(&v).unwrap().contains('\u{0}'));
    }

    #[test]
    fn timestamps_parse_in_both_offsets() {
        assert!(parse_ts("2026-07-29T10:00:00Z").is_ok());
        assert!(parse_ts("2026-07-29T10:00:00.123456+00:00").is_ok());
        assert!(parse_ts("yesterday").is_err());
    }
}
