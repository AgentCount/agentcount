//! All the database code the indexer needs, in one place.
//!
//! Keeping SQL isolated here means the ingest loop reads like a story and every
//! query lives somewhere you can find. Like the other crates we use sqlx's
//! *runtime* queries so the crate compiles without a live database (see the note
//! in `enricher/src/store.rs` for how to switch to compile-time-checked macros).
//!
//! Rust concept spotlight: **the connection pool as cheap-to-clone shared
//! state.** A `PgPool` is reference-counted internally, so `db.clone()` (done in
//! `main` to give each chain loop its own handle) shares one pool rather than
//! opening new connections.

use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::bindings::{IndexedLog, RegistryEvent};

/// Thin, cloneable wrapper over the pool.
#[derive(Clone)]
pub struct Db {
    pool: PgPool,
}

impl Db {
    /// Open a Postgres connection pool. Called once at startup; cloned per chain.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .context("connecting to Postgres")?;
        Ok(Self { pool })
    }

    /// The chains the indexer should follow, straight from the `chains` table.
    pub async fn load_enabled_chains(&self) -> Result<Vec<crate::chains::ChainConfig>> {
        let rows = sqlx::query_as::<_, crate::chains::ChainConfig>(
            "SELECT chain, chain_id, identity_registry, reputation_registry, \
                    validation_registry, deploy_block, confirmations \
             FROM chains WHERE enabled",
        )
        .fetch_all(&self.pool)
        .await
        .context("loading enabled chains")?;
        Ok(rows)
    }

    /// The last block we FULLY processed for a chain, if any. Resumption
    /// arithmetic (start at the NEXT block, or at deploy_block on first run)
    /// lives in `ingest::resume_from`, where it's unit-tested.
    pub async fn load_cursor(&self, chain: &str) -> Result<Option<i64>> {
        let last: Option<i64> =
            sqlx::query_scalar("SELECT last_block FROM indexer_cursor WHERE chain = $1")
                .bind(chain)
                .fetch_optional(&self.pool)
                .await
                .context("loading cursor")?;
        Ok(last)
    }

    /// Persist one batch atomically: raw logs, decoded rows, and the new cursor —
    /// all in a single transaction. Atomicity matters: advancing the cursor past
    /// events we didn't save (or vice-versa) would corrupt the pipeline. All-or-
    /// nothing removes that whole class of bug.
    pub async fn write_batch(
        &self,
        chain: &str,
        logs: &[IndexedLog],
        new_cursor: u64,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        for il in logs {
            // Audit log. `ON CONFLICT DO NOTHING` on (chain, tx_hash, log_index)
            // makes re-indexing a range harmless (idempotent inserts).
            sqlx::query(
                "INSERT INTO raw_events \
                    (chain, contract, event_name, block, tx_hash, block_hash, log_index, payload) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (chain, tx_hash, log_index) DO NOTHING",
            )
            .bind(&il.chain)
            .bind(&il.contract)
            .bind(il.event_name)
            .bind(il.block)
            .bind(&il.tx_hash)
            .bind(&il.block_hash)
            .bind(il.log_index)
            .bind(&il.payload)
            .execute(&mut *tx)
            .await?;

            // Route each decoded event to its typed table. The `match` is
            // exhaustive: add a variant to `RegistryEvent` and the compiler makes
            // you handle it here.
            match &il.event {
                RegistryEvent::Registered {
                    agent_id,
                    agent_uri,
                    owner,
                } => {
                    // `owner` (NFT holder) → address; `agent_uri` (metadata
                    // pointer) → the `domain` column (see 0007's note on the name).
                    sqlx::query(
                        "INSERT INTO agents \
                            (chain, agent_id, address, domain, registered_block, registered_at, registered_tx) \
                         VALUES ($1, $2, $3, $4, $5, $6, $7) \
                         ON CONFLICT (chain, agent_id) DO NOTHING",
                    )
                    .bind(&il.chain)
                    .bind(*agent_id as i64)
                    .bind(owner)
                    .bind(agent_uri)
                    .bind(il.block)
                    .bind(il.timestamp)
                    .bind(&il.tx_hash)
                    .execute(&mut *tx)
                    .await?;
                }
                RegistryEvent::Feedback {
                    to_agent_id,
                    client_address,
                    feedback_index,
                    value,
                    value_decimals,
                } => {
                    sqlx::query(
                        "INSERT INTO feedback \
                            (chain, to_agent_id, client_address, feedback_index, value, value_decimals, block, tx_hash, log_index, created_at) \
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
                         ON CONFLICT (chain, tx_hash, log_index) DO NOTHING",
                    )
                    .bind(&il.chain)
                    .bind(*to_agent_id as i64)
                    .bind(client_address)
                    .bind(*feedback_index)
                    .bind(value)
                    .bind(*value_decimals)
                    .bind(il.block)
                    .bind(&il.tx_hash)
                    .bind(il.log_index)
                    .bind(il.timestamp)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }

        // Move the cursor forward. One row per chain, upserted.
        sqlx::query(
            "INSERT INTO indexer_cursor (chain, last_block, updated_at) \
             VALUES ($1, $2, now()) \
             ON CONFLICT (chain) DO UPDATE SET last_block = EXCLUDED.last_block, updated_at = now()",
        )
        .bind(chain)
        .bind(new_cursor as i64)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }
}
