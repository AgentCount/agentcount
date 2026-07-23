//! Database access for the enricher: read agents, write enrichment + clusters.
//!
//! Same idea as the indexer's `store.rs` — all SQL in one module. The enricher
//! is mostly a *reader* of what the indexer wrote and a *writer* of derived
//! facts, so it's a good place to see sqlx's `query_as!` map rows straight into
//! your own structs.

use anyhow::Result;

use crate::clustering::Cluster;
use crate::metadata::{AgentCard, AgentStub};
use crate::liveness::ProbeOutcome;

/// Thin wrapper over the connection pool, cloneable and shareable.
#[derive(Clone)]
pub struct Db {
    // pool: sqlx::PgPool,
    pool: PoolPlaceholder,
}

/// Open the pool. Mirrors the indexer's `connect`.
pub async fn connect(database_url: &str) -> Result<Db> {
    //     let pool = sqlx::postgres::PgPoolOptions::new()
    //         .max_connections(5)
    //         .connect(database_url)
    //         .await?;
    //     Ok(Db { pool })
    let _ = database_url;
    todo!("open a PgPool and wrap it in Db")
}

impl Db {
    /// Load the agents that are due for (re-)enrichment.
    ///
    /// A good policy: agents never enriched, plus agents whose last probe is
    /// older than some interval. Start simple ("all agents") and refine.
    pub async fn load_agents_to_enrich(&self) -> Result<Vec<AgentStub>> {
        // `query_as!` maps each row directly into an `AgentStub` by matching
        // column names to field names — checked against the DB at compile time:
        //
        //     let agents = sqlx::query_as!(
        //         AgentStub,
        //         "SELECT agent_id, domain FROM agents \
        //          WHERE last_enriched_at IS NULL \
        //             OR last_enriched_at < now() - interval '6 hours'"
        //     )
        //     .fetch_all(&self.pool)
        //     .await?;
        //     Ok(agents)
        //
        // (For `query_as!` to work, `AgentStub`'s fields must line up with the
        // selected columns' names and types — another place a schema/typo
        // mistake becomes a compile error.)
        todo!("SELECT agents needing enrichment, mapping rows into AgentStub")
    }

    /// Persist the results of probing/fetching a batch of agents.
    ///
    /// The tuple shape `(AgentStub, Result<AgentCard>, ProbeOutcome)` mirrors what
    /// the concurrent probe stage in `main` produces: for each agent, whatever
    /// its metadata fetch and liveness probe returned. A failed fetch is stored
    /// as "no valid card / endpoint down", not skipped.
    pub async fn write_enrichment(
        &self,
        results: &[(AgentStub, Result<AgentCard>, ProbeOutcome)],
    ) -> Result<()> {
        // For each result: UPSERT into `agent_enrichment` (endpoint_status from
        // `outcome.is_success()`, the parsed card as jsonb, `last_enriched_at =
        // now()`), and INSERT a row into a probe-history table so the scorer can
        // compute a success *rate* over time.
        let _ = results;
        todo!("UPSERT agent_enrichment + append probe history for each result")
    }

    /// Persist detected clusters and the per-agent suspicion the scorer reads.
    pub async fn write_clusters(&self, clusters: &[Cluster]) -> Result<()> {
        // In a transaction: clear the previous clustering, then for each cluster
        // INSERT a `clusters` row and one `cluster_members` row per agent, and
        // update each member agent's `suspicion`. Replacing wholesale each pass
        // keeps the picture consistent as the graph evolves.
        let _ = clusters;
        todo!("replace clusters + cluster_members and update per-agent suspicion")
    }
}

/// Placeholder for `sqlx::PgPool`. Delete once sqlx is wired in.
#[derive(Clone)]
struct PoolPlaceholder;
