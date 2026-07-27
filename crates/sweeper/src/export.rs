//! Write the run to `data/<run_id>/` — the citable, diffable artefact.
//!
//! Postgres is the store; this is what a reader downloads, a journalist cites,
//! and a future maintainer diffs between runs. It is generated from the same
//! values that were persisted, so the two cannot disagree.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Serialize)]
pub struct RunManifest<'a> {
    pub run_id: String,
    pub chain: &'a str,
    pub chain_id: u64,
    pub registry: &'a str,
    pub pinned_block: u64,
    pub started_at: String,
    pub schema_version: i32,
    pub checker_version: &'a str,
    pub checker_commit: &'a str,
    pub spec_commit: &'a str,
    pub rerun_command: &'a str,
    pub agent_count: usize,
}

#[derive(Serialize)]
pub struct AgentDocument<'a> {
    pub run_id: String,
    pub chain: &'a str,
    pub agent_id: u64,
    pub token_id: String,
    pub owner: &'a str,
    pub agent_uri: &'a str,
    pub block_number: u64,
    pub checks: &'a [checks::CheckResult],
    /// Repeated per agent on purpose: a single agent file handed to someone
    /// must be self-describing without the manifest beside it.
    pub checker_commit: &'a str,
    pub spec_commit: &'a str,
}

pub fn run_dir(run_id: &str) -> PathBuf {
    PathBuf::from("data").join(run_id)
}

pub fn write_manifest(m: &RunManifest) -> Result<()> {
    let dir = run_dir(&m.run_id);
    std::fs::create_dir_all(&dir).context("creating run dir")?;
    let path = dir.join("manifest.json");
    std::fs::write(&path, serde_json::to_vec_pretty(m)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

pub fn write_agent(doc: &AgentDocument) -> Result<()> {
    let dir = run_dir(&doc.run_id).join(doc.chain);
    std::fs::create_dir_all(&dir).context("creating agent dir")?;
    let path = dir.join(format!("{}.json", doc.agent_id));
    std::fs::write(&path, serde_json::to_vec_pretty(doc)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}
