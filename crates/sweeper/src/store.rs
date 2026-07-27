//! Persistence for one run. Everything here is INSERT-only: a run's results
//! are never updated, because a changed result with the same run_id would
//! make the archive lie.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

pub struct Db {
    pub pool: PgPool,
}

pub struct RunMeta {
    pub run_id: Uuid,
    pub chain: String,
    pub schema_version: i32,
    pub checker_version: String,
    pub checker_commit: String,
    pub spec_commit: String,
    pub rerun_command: String,
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
            "INSERT INTO runs (run_id, chain, schema_version, checker_version, \
                               checker_commit, spec_commit, rerun_command) \
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(m.run_id)
        .bind(&m.chain)
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

    pub async fn write_snapshot(
        &self,
        run_id: Uuid,
        chain: &str,
        s: &chain::AgentSnapshot,
    ) -> Result<()> {
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
        .bind(&s.agent_uri)
        .bind(s.block_number as i64)
        .execute(&self.pool)
        .await
        .context("writing snapshot")?;
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
