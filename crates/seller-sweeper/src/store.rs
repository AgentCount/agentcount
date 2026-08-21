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
        claims: &[sellers::consistent::Claim],
        checked_at: DateTime<Utc>,
    ) -> Result<()> {
        // Claims are indexed by (payee, resource) so rung 1's evidence can
        // carry each seller's own, and rung 7 can read them back without a
        // second catalog fetch.
        let mut claims_by_seller: std::collections::HashMap<
            (&str, &str),
            Vec<&sellers::consistent::Claim>,
        > = std::collections::HashMap::new();
        for claim in claims {
            claims_by_seller
                .entry((claim.pay_to.as_str(), claim.resource.as_str()))
                .or_default()
                .push(claim);
        }
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
            // The catalog's own claims travel with rung 1, because "what the
            // catalog said" IS the evidence of being listed — and rung 7
            // compares exactly that against what the endpoint quotes.
            let seller_claims: Vec<&sellers::consistent::Claim> = resources
                .iter()
                .filter_map(|r| claims_by_seller.get(&(seller.id.pay_to.as_str(), r.as_str())))
                .flatten()
                .copied()
                .collect();
            let evidence = serde_json::json!({
                "catalogs": catalogs,
                "resource_count": resources.len(),
                "claims": seller_claims,
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

/// One sweep's metadata — what it swept, with what rules, asking what.
#[derive(Debug, Clone)]
pub struct RunMeta {
    pub network: String,
    pub checker: String,
    pub catalogs: Vec<String>,
    pub rungs_attempted: Option<Vec<i16>>,
    pub status: String,
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

    /// The catalog claims stored with a seller's rung 1 — what the catalogs
    /// said this seller charges. Read back by the probe pass so rung 7 can
    /// compare claim against quote without re-fetching any catalog.
    pub async fn claims_for(
        &self,
        run_id: Uuid,
        pay_to: &str,
        host: &str,
    ) -> Result<Vec<sellers::consistent::Claim>> {
        let row = sqlx::query(
            "SELECT evidence FROM seller_check_results \
             WHERE run_id = $1 AND pay_to = $2 AND host = $3 AND rung = 1",
        )
        .bind(run_id)
        .bind(pay_to)
        .bind(host)
        .fetch_optional(&self.pool)
        .await
        .context("reading rung 1 evidence")?;
        let Some(row) = row else {
            return Ok(Vec::new());
        };
        let evidence: serde_json::Value = row.get("evidence");
        Ok(evidence
            .get("claims")
            .and_then(|c| serde_json::from_value(c.clone()).ok())
            .unwrap_or_default())
    }

    /// Which rungs a sweep set out to ask, recorded when it opens so that a
    /// rung with no rows is legible as "never attempted" rather than as a
    /// pass that crashed.
    pub async fn record_rungs_attempted(&self, run_id: Uuid, rungs: &[i16]) -> Result<()> {
        sqlx::query("UPDATE seller_runs SET rungs_attempted = $2 WHERE run_id = $1")
            .bind(run_id)
            .bind(rungs)
            .execute(&self.pool)
            .await
            .context("recording rungs attempted")?;
        Ok(())
    }

    /// One sweep's metadata, for the delta's confound columns.
    pub async fn run_meta(&self, run_id: Uuid) -> Result<RunMeta> {
        let row = sqlx::query(
            "SELECT network, seller_checker_version, catalogs, rungs_attempted, status \
             FROM seller_runs WHERE run_id = $1",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await
        .context("reading run metadata")?;
        Ok(RunMeta {
            network: row.get("network"),
            checker: row.get("seller_checker_version"),
            catalogs: row.get("catalogs"),
            rungs_attempted: row.get("rungs_attempted"),
            status: row.get("status"),
        })
    }

    /// The sweep before this one on the same network — the pair the delta
    /// names, chosen once and then recorded permanently, so "previous" can
    /// never silently re-bind as later sweeps land.
    pub async fn previous_run(&self, run_id: Uuid, network: &str) -> Result<Option<Uuid>> {
        let row = sqlx::query(
            "SELECT r.run_id FROM seller_runs r \
             WHERE r.network = $2 AND r.run_id <> $1 \
               AND r.started_at < (SELECT started_at FROM seller_runs WHERE run_id = $1) \
             ORDER BY r.started_at DESC LIMIT 1",
        )
        .bind(run_id)
        .bind(network)
        .fetch_optional(&self.pool)
        .await
        .context("finding the previous sweep")?;
        Ok(row.map(|r| r.get::<Uuid, _>("run_id")))
    }

    /// Every `(seller, rung) -> status` a sweep recorded — the input the
    /// delta arithmetic compares.
    pub async fn rung_statuses(&self, run_id: Uuid) -> Result<sellers::delta::RungStatuses> {
        let rows = sqlx::query(
            "SELECT pay_to, host, rung, status FROM seller_check_results WHERE run_id = $1",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .context("reading rung statuses")?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    (
                        sellers::identity::SellerId {
                            pay_to: r.get("pay_to"),
                            host: r.get("host"),
                        },
                        r.get::<i16, _>("rung"),
                    ),
                    r.get::<String, _>("status"),
                )
            })
            .collect())
    }

    /// Write one sweep's delta, replacing any earlier computation of it.
    #[allow(clippy::too_many_arguments)]
    pub async fn write_delta(
        &self,
        run_id: Uuid,
        previous_run_id: Uuid,
        network: &str,
        d: &sellers::delta::SellerDelta,
        before: &RunMeta,
        after: &RunMeta,
        confound: &sellers::delta::MethodConfound,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO seller_run_deltas \
               (run_id, previous_run_id, network, sellers_before, sellers_after, \
                appeared, disappeared, came_back, went_dark, excluded_refused, \
                excluded_error, excluded_unprobed, flips, checker_before, checker_after, \
                catalogs_before, catalogs_after, rungs_before, rungs_after, method_changed) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20) \
             ON CONFLICT (run_id) DO UPDATE SET \
               previous_run_id = EXCLUDED.previous_run_id, \
               sellers_before = EXCLUDED.sellers_before, sellers_after = EXCLUDED.sellers_after, \
               appeared = EXCLUDED.appeared, disappeared = EXCLUDED.disappeared, \
               came_back = EXCLUDED.came_back, went_dark = EXCLUDED.went_dark, \
               excluded_refused = EXCLUDED.excluded_refused, \
               excluded_error = EXCLUDED.excluded_error, \
               excluded_unprobed = EXCLUDED.excluded_unprobed, \
               flips = EXCLUDED.flips, checker_before = EXCLUDED.checker_before, \
               checker_after = EXCLUDED.checker_after, catalogs_before = EXCLUDED.catalogs_before, \
               catalogs_after = EXCLUDED.catalogs_after, rungs_before = EXCLUDED.rungs_before, \
               rungs_after = EXCLUDED.rungs_after, method_changed = EXCLUDED.method_changed, \
               computed_at = now()",
        )
        .bind(run_id)
        .bind(previous_run_id)
        .bind(network)
        .bind(d.sellers_before as i32)
        .bind(d.sellers_after as i32)
        .bind(d.appeared as i32)
        .bind(d.disappeared as i32)
        .bind(i32::try_from(d.came_back).unwrap_or(i32::MAX))
        .bind(i32::try_from(d.went_dark).unwrap_or(i32::MAX))
        .bind(i32::try_from(d.excluded_refused).unwrap_or(i32::MAX))
        .bind(i32::try_from(d.excluded_error).unwrap_or(i32::MAX))
        .bind(i32::try_from(d.excluded_unprobed).unwrap_or(i32::MAX))
        .bind(serde_json::to_value(&d.flips)?)
        .bind(&before.checker)
        .bind(&after.checker)
        .bind(&before.catalogs)
        .bind(&after.catalogs)
        .bind(before.rungs_attempted.as_deref())
        .bind(after.rungs_attempted.as_deref())
        .bind(confound.changed())
        .execute(&self.pool)
        .await
        .context("writing a seller delta")?;
        Ok(())
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
