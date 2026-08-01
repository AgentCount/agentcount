//! # export-run — rebuild a run's `data/<run_id>/` export from Postgres.
//!
//! ```text
//! DATABASE_URL=… export-run <run_id>
//! ```
//!
//! The sweeper writes an export as it goes. This rebuilds one afterwards, from
//! the rows that were persisted, which is what makes the four canonical runs
//! publishable at all — they were swept before anything published exports, so
//! their directories no longer exist and the database is the only copy.
//!
//! It is also a claim worth being able to test. `export.rs` says its files are
//! "generated from the same values that were persisted, so the two cannot
//! disagree". That is only checkable if something can regenerate them.
//!
//! ## A rebuilt manifest is honest about being rebuilt
//!
//! Two of the manifest's fields cannot be recovered from the database:
//! `unreadable` and `unwritable` are counted in the sweeping process and never
//! stored. A rebuild therefore writes them as `null` and sets `rebuilt_at`,
//! rather than writing zero — which would assert that nothing was lost, when
//! what is actually true is that we no longer know.
//!
//! That means a rebuilt archive does not hash identically to the one the
//! original sweep would have written. Also fine, and also stated: the hash
//! attests the bytes published, not a counterfactual.

use anyhow::{Context, Result};
use serde::Serialize;
use sweeper::store;
use uuid::Uuid;

/// The manifest, plus the one field that says this file was not written by the
/// sweep that produced the run.
#[derive(Serialize)]
struct RebuiltManifest {
    run_id: String,
    chain: String,
    chain_id: i64,
    registry: String,
    pinned_block: Option<i64>,
    started_at: String,
    schema_version: i32,
    checker_version: String,
    checker_commit: String,
    spec_commit: String,
    rerun_command: String,
    agent_count: Option<i32>,
    swept: usize,
    /// `null`, always, on a rebuild — see the module doc. Never `0`.
    unreadable: Option<usize>,
    unwritable: Option<usize>,
    finished_at: Option<String>,
    /// Set only on a rebuilt manifest. Its presence is the signal that
    /// `unreadable`/`unwritable` are unknown rather than zero.
    rebuilt_at: String,
}

#[derive(Serialize)]
struct AgentFile {
    run_id: String,
    chain: String,
    agent_id: i64,
    token_id: String,
    owner: String,
    agent_uri: String,
    block_number: i64,
    minter: Option<String>,
    registration_tx_hash: Option<String>,
    registration_block: Option<i64>,
    checks: serde_json::Value,
    checker_commit: String,
    spec_commit: String,
    http_status: Option<i32>,
    content_type: Option<String>,
    body_bytes: Option<i32>,
    body_sha256: Option<String>,
    final_url: Option<String>,
}

/// The `runs` row, as its columns come back.
type RunRow = (
    String,                                // chain
    chrono::DateTime<chrono::Utc>,         // started_at
    Option<chrono::DateTime<chrono::Utc>>, // finished_at
    i32,                                   // schema_version
    String,                                // checker_version
    String,                                // checker_commit
    String,                                // spec_commit
    String,                                // rerun_command
    Option<i32>,                           // agent_count
    Option<i64>,                           // pinned_block
    String,                                // status
);

/// `agent_snapshots`: id, token, owner, uri, block, and the schema-6 minter
/// columns.
type SnapshotRow = (
    i64,
    String,
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
    Option<i64>,
);

/// `check_results`: one rung's answer for one agent.
type CheckRow = (
    i64,
    i16,
    String,
    String,
    serde_json::Value,
    chrono::DateTime<chrono::Utc>,
);

/// `http_archive`, summary columns only — never `body`.
type ArchiveRow = (
    i64,
    Option<i32>,
    Option<String>,
    Option<i32>,
    Option<String>,
    Option<String>,
);

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let run_id: Uuid = std::env::args()
        .nth(1)
        .context("usage: export-run <run_id>")?
        .parse()
        .context("that is not a run id")?;
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = store::Db::connect(&database_url).await?;

    let run: RunRow = sqlx::query_as(
        "SELECT chain, started_at, finished_at, schema_version, checker_version, \
                checker_commit, spec_commit, rerun_command, agent_count, pinned_block, status \
           FROM runs WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&db.pool)
    .await
    .with_context(|| format!("no run {run_id}"))?;

    let chain = run.0.clone();
    if run.10 != "finished" {
        // Publishing a run that did not finish would put an archive at a
        // permanent URL for a measurement that was interrupted. The manifest
        // would say so, but the URL would not, and the URL is what gets cited.
        anyhow::bail!(
            "run {run_id} has status {:?}, not 'finished' — refusing to export",
            run.10
        );
    }

    let (chain_id, registry, _, _) = db.chain_config(&chain).await?;

    tracing::info!("rebuilding export for run {run_id} ({chain})");

    // One query per table, joined in memory by agent id. Simpler than a wide
    // join with three left sides, and the largest run is 244,208 agents —
    // large, but not large enough to be worth streaming for a tool that runs
    // once per run.
    // `token_id::text`, not `token_id`. The column is NUMERIC because a token
    // id is a uint256 and does not fit any integer type Postgres has — and it
    // must stay a STRING all the way to the JSON file for the same reason a
    // document's `agentId` is read as one (see `rung5_bound`): a JSON number
    // loses precision above 2^53, so serialising a large id as a number would
    // publish a different id than the chain holds.
    let snapshots: Vec<SnapshotRow> = sqlx::query_as(
        "SELECT agent_id, token_id::text, owner, agent_uri, block_number, \
                    minter, registration_tx_hash, registration_block \
               FROM agent_snapshots WHERE run_id = $1 ORDER BY agent_id",
    )
    .bind(run_id)
    .fetch_all(&db.pool)
    .await
    .context("loading snapshots")?;

    let checks: Vec<CheckRow> = sqlx::query_as(
        "SELECT agent_id, rung, name, status, evidence, checked_at \
               FROM check_results WHERE run_id = $1 ORDER BY agent_id, rung",
    )
    .bind(run_id)
    .fetch_all(&db.pool)
    .await
    .context("loading check results")?;

    let archives: Vec<ArchiveRow> = sqlx::query_as(
        "SELECT agent_id, http_status, content_type, body_bytes, body_sha256, final_url \
               FROM http_archive WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_all(&db.pool)
    .await
    .context("loading archive summaries")?;

    let mut checks_by_agent: std::collections::HashMap<i64, Vec<serde_json::Value>> =
        std::collections::HashMap::new();
    for (agent_id, rung, name, status, evidence, checked_at) in checks {
        checks_by_agent
            .entry(agent_id)
            .or_default()
            .push(serde_json::json!({
                "rung": rung,
                "name": name,
                "status": status,
                "evidence": evidence,
                "checked_at": checked_at.to_rfc3339(),
            }));
    }
    let archive_by_agent: std::collections::HashMap<i64, _> =
        archives.into_iter().map(|a| (a.0, a)).collect();

    let dir = std::path::Path::new("data")
        .join(run_id.to_string())
        .join(&chain);
    std::fs::create_dir_all(&dir).context("creating the export directory")?;

    let swept = snapshots.len();
    for (agent_id, token_id, owner, agent_uri, block_number, minter, reg_tx, reg_block) in snapshots
    {
        let a = archive_by_agent.get(&agent_id);
        let file = AgentFile {
            run_id: run_id.to_string(),
            chain: chain.clone(),
            agent_id,
            token_id,
            owner,
            agent_uri,
            block_number,
            minter,
            registration_tx_hash: reg_tx,
            registration_block: reg_block,
            checks: serde_json::Value::Array(checks_by_agent.remove(&agent_id).unwrap_or_default()),
            checker_commit: run.5.clone(),
            spec_commit: run.6.clone(),
            http_status: a.and_then(|a| a.1),
            content_type: a.and_then(|a| a.2.clone()),
            body_bytes: a.and_then(|a| a.3),
            body_sha256: a.and_then(|a| a.4.clone()),
            final_url: a.and_then(|a| a.5.clone()),
        };
        std::fs::write(
            dir.join(format!("{agent_id}.json")),
            serde_json::to_vec_pretty(&file)?,
        )
        .with_context(|| format!("writing agent {agent_id}"))?;
    }

    let manifest = RebuiltManifest {
        run_id: run_id.to_string(),
        chain: chain.clone(),
        chain_id,
        registry,
        pinned_block: run.9,
        started_at: run.1.to_rfc3339(),
        schema_version: run.3,
        checker_version: run.4,
        checker_commit: run.5,
        spec_commit: run.6,
        rerun_command: run.7,
        agent_count: run.8,
        swept,
        unreadable: None,
        unwritable: None,
        finished_at: run.2.map(|t| t.to_rfc3339()),
        rebuilt_at: chrono::Utc::now().to_rfc3339(),
    };
    std::fs::write(
        std::path::Path::new("data")
            .join(run_id.to_string())
            .join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )
    .context("writing the manifest")?;

    tracing::info!(
        "wrote data/{run_id}/ — {swept} agents, manifest marked rebuilt \
         (unreadable/unwritable are null, not zero)"
    );
    Ok(())
}
