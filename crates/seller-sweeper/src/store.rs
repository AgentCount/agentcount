//! The writes, against migration 0026.
//!
//! Nothing here decides anything: every value written was computed by
//! `crates/sellers` or observed by [`crate::fetcher`]. The one thing this
//! module enforces is that a run's rows arrive TOGETHER — a population
//! without its catalog snapshots is not reproducible, and a snapshot without
//! its population is not a census.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sellers::catalog::Population;
use sqlx::{Postgres, Row, postgres::PgPoolOptions};
use uuid::Uuid;

use crate::fetcher::Fetched;

pub struct Db {
    pool: sqlx::Pool<Postgres>,
}

impl Db {
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(url)
            .await
            .context("connecting to Postgres")?;
        Ok(Self { pool })
    }

    /// Open a run. The catalog list is stored WITH it, because the list is
    /// part of the method: a seller that vanishes next week because its only
    /// catalog was dropped is a method change, not churn, and the comparison
    /// that tells those apart reads this column.
    pub async fn open_run(
        &self,
        run_id: Uuid,
        network: &str,
        catalogs: &[String],
        checker_commit: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO seller_runs \
               (run_id, network, started_at, status, seller_checker_version, \
                checker_commit, catalogs) \
             VALUES ($1, $2, now(), 'running', $3, $4, $5)",
        )
        .bind(run_id)
        .bind(network)
        .bind(sellers::SELLER_CHECKER_VERSION)
        .bind(checker_commit)
        .bind(catalogs)
        .execute(&self.pool)
        .await
        .context("opening a seller run")?;
        Ok(())
    }

    /// Record what one catalog served — including when it refused us or fell
    /// over. A catalog the run attempted always gets a row; absence of a row
    /// means it was not in this run's list at all.
    pub async fn write_snapshot(
        &self,
        run_id: Uuid,
        catalog: &str,
        fetched: &Fetched,
        listing_count: Option<i32>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO seller_catalog_snapshots \
               (run_id, catalog, url, fetched_at, outcome, http_status, \
                sha256, byte_len, listing_count, note) \
             VALUES ($1, $2, $3, now(), $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (run_id, catalog, url) DO UPDATE SET \
               fetched_at = EXCLUDED.fetched_at, outcome = EXCLUDED.outcome, \
               http_status = EXCLUDED.http_status, sha256 = EXCLUDED.sha256, \
               byte_len = EXCLUDED.byte_len, listing_count = EXCLUDED.listing_count, \
               note = EXCLUDED.note",
        )
        .bind(run_id)
        .bind(catalog)
        .bind(&fetched.url)
        .bind(fetched.outcome.as_str())
        .bind(fetched.http_status.map(i32::from))
        .bind(fetched.sha256.as_deref())
        .bind(fetched.byte_len)
        .bind(listing_count)
        .bind(fetched.note.as_deref())
        .execute(&self.pool)
        .await
        .context("writing a catalog snapshot")?;
        Ok(())
    }

    /// Write the assembled population, its rejected listings, and rung 1 —
    /// in ONE transaction. A population half-written is a population with a
    /// wrong denominator, and every rate this instrument publishes is stated
    /// over that denominator.
    pub async fn write_population(
        &self,
        run_id: Uuid,
        population: &Population,
        checked_at: DateTime<Utc>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        for seller in &population.sellers {
            let catalogs: Vec<String> = seller.catalogs.iter().cloned().collect();
            let resources: Vec<String> = seller.resources.iter().cloned().collect();
            sqlx::query(
                "INSERT INTO seller_population (run_id, pay_to, host, catalogs, resources) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (run_id, pay_to, host) DO UPDATE SET \
                   catalogs = EXCLUDED.catalogs, resources = EXCLUDED.resources",
            )
            .bind(run_id)
            .bind(&seller.id.pay_to)
            .bind(&seller.id.host)
            .bind(&catalogs)
            .bind(&resources)
            .execute(&mut *tx)
            .await
            .context("writing a seller")?;

            // Rung 1 is `listed`, and it is evidence rather than a verdict:
            // the population IS the listed, so the answer is always `pass`
            // and the interesting part is which catalogs, and how many
            // resources they named.
            let evidence = serde_json::json!({
                "catalogs": catalogs,
                "resource_count": resources.len(),
            });
            sqlx::query(
                "INSERT INTO seller_check_results \
                   (run_id, pay_to, host, rung, name, status, evidence, checked_at) \
                 VALUES ($1, $2, $3, 1, 'listed', 'pass', $4, $5) \
                 ON CONFLICT (run_id, pay_to, host, rung) DO UPDATE SET \
                   status = EXCLUDED.status, evidence = EXCLUDED.evidence, \
                   checked_at = EXCLUDED.checked_at",
            )
            .bind(run_id)
            .bind(&seller.id.pay_to)
            .bind(&seller.id.host)
            .bind(&evidence)
            .bind(checked_at)
            .execute(&mut *tx)
            .await
            .context("writing rung 1")?;
        }

        // §10.2's losslessness, as rows. The catalog's own text, unnormalized.
        for rejected in &population.rejected {
            sqlx::query(
                "INSERT INTO seller_rejected_listings (run_id, catalog, pay_to, resource, reason) \
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(run_id)
            .bind(&rejected.listing.catalog)
            .bind(&rejected.listing.pay_to)
            .bind(&rejected.listing.resource)
            .bind(&rejected.reason)
            .execute(&mut *tx)
            .await
            .context("writing a rejected listing")?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Close a run. `finished` only when the caller can say the population is
    /// complete; a run that could not read every catalog in its list is
    /// `failed`, because a smaller population that reads as a complete one is
    /// the worst thing this table could hold.
    pub async fn close_run(&self, run_id: Uuid, status: &str, seller_count: i32) -> Result<()> {
        sqlx::query(
            "UPDATE seller_runs SET status = $2, finished_at = now(), seller_count = $3 \
             WHERE run_id = $1",
        )
        .bind(run_id)
        .bind(status)
        .bind(seller_count)
        .execute(&self.pool)
        .await
        .context("closing a seller run")?;
        Ok(())
    }

    /// Clear a run's rows before rewriting them, so a re-run of the same
    /// `run_id` replaces rather than accumulates. Only ever called against a
    /// run this process opened.
    pub async fn clear_run(&self, run_id: Uuid) -> Result<()> {
        for table in [
            "seller_check_results",
            "seller_population",
            "seller_rejected_listings",
        ] {
            sqlx::query(&format!("DELETE FROM {table} WHERE run_id = $1"))
                .bind(run_id)
                .execute(&self.pool)
                .await
                .with_context(|| format!("clearing {table}"))?;
        }
        Ok(())
    }

    /// How many sellers a run recorded — read back rather than remembered,
    /// so the number a run reports is the number it stored.
    pub async fn seller_count(&self, run_id: Uuid) -> Result<i64> {
        let row = sqlx::query("SELECT count(*) AS n FROM seller_population WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&self.pool)
            .await
            .context("counting sellers")?;
        Ok(row.get::<i64, _>("n"))
    }
}

/// One seller as stored, with the resources a probe may choose from.
#[derive(Debug, Clone)]
pub struct StoredSeller {
    pub pay_to: String,
    pub host: String,
    pub resources: Vec<String>,
}

impl Db {
    /// The most recent seller run for a network, whatever its status — the
    /// probe pass names the run it is extending rather than guessing, and a
    /// caller that wants a different one passes its id.
    pub async fn latest_run(&self, network: &str) -> Result<Option<Uuid>> {
        let row = sqlx::query(
            "SELECT run_id FROM seller_runs WHERE network = $1 \
             ORDER BY started_at DESC LIMIT 1",
        )
        .bind(network)
        .fetch_optional(&self.pool)
        .await
        .context("finding the latest seller run")?;
        Ok(row.map(|r| r.get::<Uuid, _>("run_id")))
    }

    /// Every seller in a run, ordered so two probe passes visit them in the
    /// same order — the same determinism the assembly keeps.
    pub async fn population_for_run(&self, run_id: Uuid) -> Result<Vec<StoredSeller>> {
        let rows = sqlx::query(
            "SELECT pay_to, host, resources FROM seller_population \
             WHERE run_id = $1 ORDER BY host, pay_to",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .context("reading the population")?;
        Ok(rows
            .into_iter()
            .map(|r| StoredSeller {
                pay_to: r.get("pay_to"),
                host: r.get("host"),
                resources: r.get("resources"),
            })
            .collect())
    }

    /// One rung's answer for one seller.
    #[allow(clippy::too_many_arguments)]
    pub async fn write_check(
        &self,
        run_id: Uuid,
        pay_to: &str,
        host: &str,
        rung: i16,
        name: &str,
        status: &str,
        reason: Option<&str>,
        evidence: &serde_json::Value,
        checked_at: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO seller_check_results \
               (run_id, pay_to, host, rung, name, status, reason, evidence, checked_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (run_id, pay_to, host, rung) DO UPDATE SET \
               status = EXCLUDED.status, reason = EXCLUDED.reason, \
               evidence = EXCLUDED.evidence, checked_at = EXCLUDED.checked_at",
        )
        .bind(run_id)
        .bind(pay_to)
        .bind(host)
        .bind(rung)
        .bind(name)
        .bind(status)
        .bind(reason)
        .bind(evidence)
        .bind(checked_at)
        .execute(&self.pool)
        .await
        .context("writing a seller check result")?;
        Ok(())
    }
}
