//! Database access for the enricher: read agents, append observations, upsert
//! flags. All SQL lives here so the rest of the crate reads like a story.
//!
//! ## A note on runtime vs. compile-time-checked queries
//!
//! We use sqlx's *runtime* query functions (`sqlx::query`, `query_as`) with
//! `.bind(...)` for parameters. These are checked when they run, not at compile
//! time, so the crate builds without a live database — friendlier for a project
//! you clone and explore. To upgrade to sqlx's famous COMPILE-time checking,
//! swap `query_as::<_, T>("...")` for the `query_as!(T, "...")` macro and run
//! `cargo sqlx prepare` against a migrated database; typos then become build
//! errors. Same SQL either way.
//!
//! Rust concept spotlight: **`#[derive(sqlx::FromRow)]`.** Put it on a struct and
//! sqlx maps a result row into it by matching column names to field names. That's
//! why every query below aliases its columns to the field names of the struct it
//! loads into.

use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::flags::{AgentFlag, AgentKey, AgentNode};
use crate::metadata::AgentStub;

/// Thin wrapper over the connection pool, cloneable and shareable.
/// `pub(crate) pool` so the sqlx tests below can construct a Db directly.
#[derive(Clone)]
pub struct Db {
    pub(crate) pool: PgPool,
}

impl Db {
    /// Open the pool. Called once at startup.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .context("connecting to Postgres")?;
        Ok(Self { pool })
    }

    /// Load the agents due for (re-)enrichment: never enriched, or stale.
    pub async fn load_agents_to_enrich(&self) -> Result<Vec<AgentStub>> {
        let agents = sqlx::query_as::<_, AgentStub>(
            "SELECT chain, agent_id, domain \
             FROM agents \
             WHERE last_enriched_at IS NULL \
                OR last_enriched_at < now() - interval '6 hours' \
             ORDER BY last_enriched_at NULLS FIRST \
             LIMIT 1000",
        )
        .fetch_all(&self.pool)
        .await
        .context("loading agents to enrich")?;
        Ok(agents)
    }

    /// Persist one batch of observations. Per agent, per pass:
    ///   * APPEND a metadata_snapshots row — success or failure, the archive
    ///     records what the domain served (or didn't) at this moment;
    ///   * APPEND a probe_history row;
    ///   * UPDATE the agent_enrichment cache (latest liveness; last GOOD card).
    /// History is never updated or deleted — it's the moat.
    pub async fn write_observations(&self, observations: &[crate::observe::Observation]) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        for o in observations {
            sqlx::query(
                "INSERT INTO metadata_snapshots (chain, agent_id, url, http_status, content_hash, body, error) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(&o.agent.chain)
            .bind(o.agent.agent_id)
            .bind(&o.url)
            .bind(o.outcome.http_status())
            .bind(&o.body_hash)
            .bind(&o.body)
            .bind(o.outcome.error_detail())
            .execute(&mut *tx)
            .await?;

            sqlx::query(
                "INSERT INTO probe_history (chain, agent_id, outcome, latency_ms) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(&o.agent.chain)
            .bind(o.agent.agent_id)
            .bind(o.outcome.label())
            .bind(o.outcome.latency_ms())
            .execute(&mut *tx)
            .await?;

            // COALESCE keeps the last GOOD card when this pass got nothing —
            // the cache serves the UI; the truth lives in the snapshots.
            let card: Option<&serde_json::Value> =
                if o.outcome.is_alive() { o.body.as_ref() } else { None };
            sqlx::query(
                "INSERT INTO agent_enrichment (chain, agent_id, agent_card, endpoint_healthy, last_probed_at) \
                 VALUES ($1, $2, $3, $4, now()) \
                 ON CONFLICT (chain, agent_id) DO UPDATE SET \
                    agent_card = COALESCE(EXCLUDED.agent_card, agent_enrichment.agent_card), \
                    endpoint_healthy = EXCLUDED.endpoint_healthy, \
                    last_probed_at = EXCLUDED.last_probed_at",
            )
            .bind(&o.agent.chain)
            .bind(o.agent.agent_id)
            .bind(card)
            .bind(o.outcome.is_alive())
            .execute(&mut *tx)
            .await?;

            sqlx::query("UPDATE agents SET last_enriched_at = now() WHERE chain = $1 AND agent_id = $2")
                .bind(&o.agent.chain)
                .bind(o.agent.agent_id)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Load every agent's clustering inputs (id, operator address, reg time).
    pub async fn load_agent_nodes(&self) -> Result<Vec<AgentNode>> {
        // A hand-mapped query (rather than FromRow) because `AgentNode` nests an
        // `AgentKey`, which sqlx can't derive a row mapping for. `query_as` into a
        // flat row, then reshape — a common, honest pattern.
        #[derive(sqlx::FromRow)]
        struct Row {
            chain: String,
            agent_id: i64,
            address: String,
            registered_at: chrono::DateTime<chrono::Utc>,
        }
        let rows = sqlx::query_as::<_, Row>(
            // address_norm, not address: grouping by operator wallet must
            // never fragment on hex casing.
            "SELECT chain, agent_id, address_norm AS address, registered_at FROM agents",
        )
        .fetch_all(&self.pool)
        .await
        .context("loading agent nodes")?;

        Ok(rows
            .into_iter()
            .map(|r| AgentNode {
                key: AgentKey {
                    chain: r.chain,
                    agent_id: r.agent_id,
                },
                address: r.address,
                registered_at: r.registered_at,
            })
            .collect())
    }

    /// Upsert flags, append-only at the EVENT level: a new (subject, kind)
    /// inserts a flag plus a 'raised' event; changed evidence updates the
    /// current row plus an 'evidence_added' event. Nothing is ever deleted —
    /// the flag_events trail is the observation history.
    pub async fn upsert_flags(&self, flags: &[AgentFlag]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for f in flags {
            let existing: Option<(i64, serde_json::Value)> = sqlx::query_as(
                "SELECT id, evidence FROM flags WHERE chain = $1 AND agent_id = $2 AND kind = $3",
            )
            .bind(&f.key.chain)
            .bind(f.key.agent_id)
            .bind(f.kind.label())
            .fetch_optional(&mut *tx)
            .await?;

            match existing {
                None => {
                    let id: i64 = sqlx::query_scalar(
                        "INSERT INTO flags (chain, agent_id, kind, evidence) \
                         VALUES ($1, $2, $3, $4) RETURNING id",
                    )
                    .bind(&f.key.chain)
                    .bind(f.key.agent_id)
                    .bind(f.kind.label())
                    .bind(&f.evidence)
                    .fetch_one(&mut *tx)
                    .await?;
                    sqlx::query("INSERT INTO flag_events (flag_id, event, detail) VALUES ($1, 'raised', $2)")
                        .bind(id)
                        .bind(&f.evidence)
                        .execute(&mut *tx)
                        .await?;
                }
                Some((id, old)) if old != f.evidence => {
                    sqlx::query("UPDATE flags SET evidence = $1 WHERE id = $2")
                        .bind(&f.evidence)
                        .bind(id)
                        .execute(&mut *tx)
                        .await?;
                    sqlx::query("INSERT INTO flag_events (flag_id, event, detail) VALUES ($1, 'evidence_added', $2)")
                        .bind(id)
                        .bind(&f.evidence)
                        .execute(&mut *tx)
                        .await?;
                }
                Some(_) => {} // unchanged — no event, no churn
            }
        }
        tx.commit().await?;
        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observe::{Observation, ProbeOutcome};

    /// `#[sqlx::test]` spins up a fresh database per test and applies every
    /// migration — the test proves code and schema agree. Needs DATABASE_URL
    /// pointing at a running Postgres.
    #[sqlx::test(migrations = "../../migrations")]
    async fn observations_append_history_and_update_cache(pool: sqlx::PgPool) {
        // Fixture agent (FK target).
        sqlx::query(
            "INSERT INTO agents (chain, agent_id, address, domain, registered_block, registered_at, registered_tx) \
             VALUES ('base', 1, '0xabc', 'agent.example', 1, now(), '0xdead')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let db = Db { pool: pool.clone() };
        let agent = AgentStub { chain: "base".into(), agent_id: 1, domain: "agent.example".into() };
        let obs = |outcome: ProbeOutcome, body: Option<serde_json::Value>| Observation {
            agent: agent.clone(),
            url: "https://agent.example/.well-known/agent.json".into(),
            body_hash: body.as_ref().map(|_| "hash1".into()),
            body,
            outcome,
        };

        // Two passes: first healthy with a card, then the endpoint dies.
        db.write_observations(&[obs(
            ProbeOutcome::Healthy { latency_ms: 50 },
            Some(serde_json::json!({"name": "A"})),
        )])
        .await
        .unwrap();
        db.write_observations(&[obs(ProbeOutcome::Unreachable, None)]).await.unwrap();

        // History: BOTH snapshots and BOTH probes kept — nothing overwritten.
        let snapshots: i64 =
            sqlx::query_scalar("SELECT count(*) FROM metadata_snapshots WHERE agent_id = 1")
                .fetch_one(&pool).await.unwrap();
        assert_eq!(snapshots, 2, "every fetch is archived, including failures");
        let probes: i64 =
            sqlx::query_scalar("SELECT count(*) FROM probe_history WHERE agent_id = 1")
                .fetch_one(&pool).await.unwrap();
        assert_eq!(probes, 2);

        // Cache: reflects the LATEST state (down), but the last good card
        // survives — the cache updates, history accumulates.
        let (healthy, card): (bool, Option<serde_json::Value>) = sqlx::query_as(
            "SELECT endpoint_healthy, agent_card FROM agent_enrichment WHERE agent_id = 1",
        )
        .fetch_one(&pool).await.unwrap();
        assert!(!healthy);
        assert_eq!(card.unwrap()["name"], "A");
    }
}
