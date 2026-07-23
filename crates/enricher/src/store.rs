//! Database access for the enricher: read agents, write enrichment, clusters,
//! and scores. All SQL lives here so the rest of the crate reads like a story.
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

use crate::clustering::{AgentKey, AgentNode, Cluster};
use crate::metadata::AgentStub;

/// Thin wrapper over the connection pool, cloneable and shareable.
/// `pub(crate) pool` so the sqlx tests below can construct a Db directly.
#[derive(Clone)]
pub struct Db {
    pub(crate) pool: PgPool,
}

/// One per-agent aggregate row for scoring (see `load_score_inputs`).
#[derive(Debug, sqlx::FromRow)]
pub struct ScoreInputRow {
    pub chain: String,
    pub agent_id: i64,
    pub first_seen: chrono::DateTime<chrono::Utc>,
    pub last_activity: chrono::DateTime<chrono::Utc>,
    pub suspicion: f64,
    pub distinct_counterparties: i64,
    pub total_payment_value: f64,
    pub active_days: i64,
    pub probe_count: i64,
    pub probe_successes: i64,
    pub cluster_size: i64,
}

/// One raw feedback edge (used to reconstruct the reputation graph in Rust).
#[derive(Debug, sqlx::FromRow)]
pub struct FeedbackRow {
    pub chain: String,
    pub from_agent_id: i64,
    pub to_agent_id: i64,
    pub score: i16,
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
            .bind(if o.outcome.is_alive() { None } else { Some(o.outcome.label()) })
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
            "SELECT chain, agent_id, address, registered_at FROM agents",
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

    /// Load all directed feedback edges as `(from, to)` agent-key pairs.
    pub async fn load_feedback_pairs(&self) -> Result<Vec<(AgentKey, AgentKey)>> {
        #[derive(sqlx::FromRow)]
        struct Row {
            chain: String,
            from_agent_id: i64,
            to_agent_id: i64,
        }
        let rows = sqlx::query_as::<_, Row>(
            "SELECT chain, from_agent_id, to_agent_id FROM feedback",
        )
        .fetch_all(&self.pool)
        .await
        .context("loading feedback pairs")?;

        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    AgentKey {
                        chain: r.chain.clone(),
                        agent_id: r.from_agent_id,
                    },
                    AgentKey {
                        chain: r.chain,
                        agent_id: r.to_agent_id,
                    },
                )
            })
            .collect())
    }

    /// Replace the clustering wholesale: wipe the old clusters, reset everyone's
    /// suspicion, then insert the freshly-detected clusters and stamp each
    /// member's suspicion. Wrapped in a transaction so readers never see a
    /// half-updated picture.
    pub async fn write_clusters(&self, clusters: &[Cluster]) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // `DELETE FROM clusters` cascades to `cluster_members` (ON DELETE CASCADE).
        sqlx::query("DELETE FROM clusters").execute(&mut *tx).await?;
        sqlx::query("UPDATE agents SET suspicion = 0")
            .execute(&mut *tx)
            .await?;

        for cluster in clusters {
            let reasons: Vec<&str> = cluster.reasons.iter().map(|r| r.label()).collect();
            let reasons_json = serde_json::to_value(&reasons)?;

            // Insert the cluster and get its generated UUID back.
            let cluster_id: uuid::Uuid = sqlx::query_scalar(
                "INSERT INTO clusters (suspicion, reasons) VALUES ($1, $2) RETURNING id",
            )
            .bind(cluster.suspicion)
            .bind(&reasons_json)
            .fetch_one(&mut *tx)
            .await?;

            for member in &cluster.members {
                sqlx::query(
                    "INSERT INTO cluster_members (cluster_id, chain, agent_id) \
                     VALUES ($1, $2, $3)",
                )
                .bind(cluster_id)
                .bind(&member.chain)
                .bind(member.agent_id)
                .execute(&mut *tx)
                .await?;

                sqlx::query(
                    "UPDATE agents SET suspicion = $1 WHERE chain = $2 AND agent_id = $3",
                )
                .bind(cluster.suspicion)
                .bind(&member.chain)
                .bind(member.agent_id)
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await?;
        Ok(())
    }

    /// Load one aggregate row per agent, joining in economic activity, probe
    /// history, and cluster size. `LEFT JOIN` + `COALESCE` means agents with no
    /// activity still get a row (with zeros), rather than being dropped.
    pub async fn load_score_inputs(&self) -> Result<Vec<ScoreInputRow>> {
        let rows = sqlx::query_as::<_, ScoreInputRow>(
            "SELECT \
                a.chain AS chain, \
                a.agent_id AS agent_id, \
                a.registered_at AS first_seen, \
                COALESCE(ea.last_activity, a.registered_at) AS last_activity, \
                a.suspicion AS suspicion, \
                COALESCE(ea.distinct_counterparties, 0) AS distinct_counterparties, \
                COALESCE(ea.total_payment_value, 0.0) AS total_payment_value, \
                COALESCE(ea.active_days, 0) AS active_days, \
                COALESCE(ph.probe_count, 0) AS probe_count, \
                COALESCE(ph.probe_successes, 0) AS probe_successes, \
                COALESCE(cl.cluster_size, 1) AS cluster_size \
             FROM agents a \
             LEFT JOIN ( \
                SELECT chain, agent_id, \
                    count(DISTINCT counterparty) AS distinct_counterparties, \
                    sum(value)::double precision AS total_payment_value, \
                    count(DISTINCT date(occurred_at)) AS active_days, \
                    max(occurred_at) AS last_activity \
                FROM economic_activity GROUP BY chain, agent_id \
             ) ea ON ea.chain = a.chain AND ea.agent_id = a.agent_id \
             LEFT JOIN ( \
                SELECT chain, agent_id, \
                    count(*) AS probe_count, \
                    count(*) FILTER (WHERE outcome = 'healthy') AS probe_successes \
                FROM probe_history GROUP BY chain, agent_id \
             ) ph ON ph.chain = a.chain AND ph.agent_id = a.agent_id \
             LEFT JOIN ( \
                SELECT cm.chain, cm.agent_id, c2.cnt AS cluster_size \
                FROM cluster_members cm \
                JOIN ( \
                    SELECT cluster_id, count(*) AS cnt \
                    FROM cluster_members GROUP BY cluster_id \
                ) c2 ON c2.cluster_id = cm.cluster_id \
             ) cl ON cl.chain = a.chain AND cl.agent_id = a.agent_id",
        )
        .fetch_all(&self.pool)
        .await
        .context("loading score inputs")?;
        Ok(rows)
    }

    /// Load all feedback rows for reputation scoring.
    pub async fn load_feedback_rows(&self) -> Result<Vec<FeedbackRow>> {
        let rows = sqlx::query_as::<_, FeedbackRow>(
            "SELECT chain, from_agent_id, to_agent_id, score FROM feedback",
        )
        .fetch_all(&self.pool)
        .await
        .context("loading feedback rows")?;
        Ok(rows)
    }

    /// Append a freshly-computed score for each agent (history is kept; the API
    /// reads the latest row per agent).
    pub async fn write_scores(&self, scores: &[(AgentKey, scoring::TrustScore)]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for (key, s) in scores {
            sqlx::query(
                "INSERT INTO scores \
                    (chain, agent_id, payment, liveness, age, reputation, sybil_penalty, final_score) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(&key.chain)
            .bind(key.agent_id)
            .bind(s.payment)
            .bind(s.liveness)
            .bind(s.age)
            .bind(s.reputation)
            .bind(s.sybil_penalty)
            .bind(s.final_score)
            .execute(&mut *tx)
            .await?;
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
