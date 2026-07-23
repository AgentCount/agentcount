//! All the database code the indexer needs, in one place.
//!
//! Keeping SQL isolated in a `store` module (rather than scattering queries
//! through the ingest loop) means the loop reads like a story — "load cursor,
//! fetch, decode, write batch" — and every query lives somewhere you can find.
//!
//! Rust concept spotlight: **the connection pool as shared state.** A
//! `sqlx::PgPool` manages a set of reusable connections. It's designed to be
//! cloned cheaply and shared across tasks — internally it's reference-counted,
//! so `db.clone()` shares the same underlying pool rather than opening new
//! connections. That's how both chain loops write through one pool safely.

use anyhow::Result;

use crate::bindings::{RawLog, RegistryEvent};

/// A thin wrapper around the sqlx pool.
///
/// We could pass `PgPool` around directly, but wrapping it in our own `Db` type
/// lets us hang exactly the methods we want off it and keeps call sites tidy.
/// The `#[derive(Clone)]` makes `db.clone()` work; it's cheap because the inner
/// pool is itself cheap to clone.
#[derive(Clone)]
pub struct Db {
    // The real field:
    //     pool: sqlx::PgPool,
    pool: PoolPlaceholder,
}

/// Open a Postgres connection pool.
///
/// Called once at startup; the resulting `Db` is cloned to each chain loop.
pub async fn connect(database_url: &str) -> Result<Db> {
    // With sqlx:
    //     let pool = sqlx::postgres::PgPoolOptions::new()
    //         .max_connections(5)
    //         .connect(database_url)
    //         .await?;
    //     Ok(Db { pool })
    let _ = database_url;
    todo!("open a PgPool and wrap it in Db")
}

impl Db {
    /// Return the block number to resume indexing from for a given chain.
    ///
    /// Reads the `indexer_cursor` table. If there's no row yet (first run), fall
    /// back to a configured deployment block so we don't scan the whole chain.
    pub async fn load_cursor(&self, chain: &str) -> Result<u64> {
        // sqlx's `query_scalar!` checks this SQL against your real database at
        // COMPILE time and infers that the result is an `i64`. A typo in the
        // column name fails `cargo build`, not production.
        //
        //     let row: Option<i64> = sqlx::query_scalar!(
        //         "SELECT last_block FROM indexer_cursor WHERE chain = $1",
        //         chain
        //     )
        //     .fetch_optional(&self.pool)   // `Option`: there may be no row yet
        //     .await?;
        //     Ok(row.map(|b| b as u64).unwrap_or(DEPLOY_BLOCK))
        let _ = chain;
        todo!("SELECT last_block FROM indexer_cursor, defaulting on first run")
    }

    /// Persist one batch atomically: raw logs, decoded events, and the new cursor
    /// value — all in a single transaction.
    ///
    /// Atomicity matters: if we saved events but the process died before moving
    /// the cursor, we'd double-write them next time; if we moved the cursor but
    /// died before saving events, we'd lose them forever. A transaction makes the
    /// whole batch all-or-nothing.
    pub async fn write_batch(
        &self,
        chain: &str,
        raw_logs: &[RawLog],
        events: &[RegistryEvent],
        new_cursor: u64,
    ) -> Result<()> {
        // Sketch:
        //     let mut tx = self.pool.begin().await?;      // BEGIN
        //     for log in raw_logs { /* INSERT INTO raw_events ... */ }
        //     for ev in events {
        //         // A `match` on the event enum routes each variant to the right
        //         // table (agents / feedback / validations). The compiler makes
        //         // sure you handle every variant.
        //         match ev {
        //             RegistryEvent::AgentRegistered { .. } => { /* upsert agents */ }
        //             RegistryEvent::FeedbackGiven    { .. } => { /* insert feedback */ }
        //             RegistryEvent::ValidationRecorded { .. } => { /* insert validations */ }
        //         }
        //     }
        //     // UPSERT the cursor to `new_cursor`.
        //     tx.commit().await?;                          // COMMIT (or rollback on drop)
        //     Ok(())
        let _ = (chain, raw_logs, events, new_cursor);
        todo!("write raw_events + decoded rows + cursor in one transaction")
    }
}

/// Placeholder for `sqlx::PgPool`. Delete once sqlx is wired in.
#[derive(Clone)]
struct PoolPlaceholder;
