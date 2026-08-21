//! # seller-crawl — enumerate the sellers, once.
//!
//! ```text
//! DATABASE_URL=… seller-crawl                 # the catalogs, for real
//! DATABASE_URL=… seller-crawl --dry-run       # fetch and report, write nothing
//! SELLER_MAX_PAGES=5 … seller-crawl           # stop early (development)
//! ```
//!
//! One pass of rung 1 (`listed`, METHODOLOGY §10.3): fetch every catalog in
//! the run's list, hash what each served, assemble the union population, and
//! write it. It probes no seller and buys nothing — rungs 2 and 3 are a
//! separate pass over a population that already exists, exactly as rung 6 is
//! a separate pass in the registration census, and for the same reason: the
//! unit of work is different.
//!
//! ## What "finished" means here
//!
//! A run is `finished` only if EVERY catalog in its list was read. If any
//! catalog refused us or fell over, the run is `failed` — not because the
//! process crashed, but because a population assembled from four of six
//! catalogs is a different population, and one that reads as complete is the
//! worst thing this database could hold. The partial rows are kept (they are
//! evidence about the catalogs), and the run says plainly that it is not a
//! census.
//!
//! ## Politeness
//!
//! robots.txt is honoured for every request, with no carve-out (§10.3), and
//! pages are fetched one at a time with a pause between them. A catalog is
//! somebody's server and this census reads all of it, weekly, forever.

use std::time::Duration;

use anyhow::{Context, Result};
use seller_sweeper::fetcher::{CATALOG_PAGE_DELAY, CatalogFetcher, Outcome};
use seller_sweeper::store::Db;
use sellers::catalog::{self, Listing};
use sellers::identity::Network;
use sellers::sources::bazaar;
use uuid::Uuid;

/// The Bazaar's page size. Its API clamps `limit`, so this is what it gives
/// rather than what we ask for; the crawler follows `pagination.total`
/// instead of assuming.
const BAZAAR_PAGE: u64 = 100;

/// A hard stop on pagination, so a catalog that reports an ever-growing
/// total (or a bug here) cannot become an unbounded crawl of somebody's
/// server. 15,155 resources at 100 a page was 152 pages when this was
/// written; 500 leaves room to grow and still ends.
const MAX_PAGES: u64 = 500;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let dry_run = std::env::args().any(|a| a == "--dry-run");
    let max_pages: u64 = std::env::var("SELLER_MAX_PAGES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MAX_PAGES)
        .min(MAX_PAGES);

    // Sweep 1 is Base/USDC (§10.5), and one catalog: the Bazaar. The list is
    // part of the method and travels with the run.
    let network = sellers::network::BASE;
    let catalogs = vec![bazaar::NAME.to_string()];

    let fetcher = CatalogFetcher::new()?;
    let run_id = Uuid::new_v4();
    let started = chrono::Utc::now();

    tracing::info!(
        "seller-crawl {} — network {}, catalogs {:?}, checker {}{}",
        run_id,
        network,
        catalogs,
        sellers::SELLER_CHECKER_VERSION,
        if dry_run { " (DRY RUN)" } else { "" }
    );

    let db = if dry_run {
        None
    } else {
        let url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
        let db = Db::connect(&url).await?;
        db.open_run(
            run_id,
            network,
            &catalogs,
            option_env!("CHECKER_COMMIT_OVERRIDE").unwrap_or("unknown"),
        )
        .await?;
        db.clear_run(run_id).await?;
        // What this sweep SET OUT to ask. The crawl asks rung 1 only; the
        // probe pass adds its own when it runs. A rung absent from this list
        // was never attempted, which is a different fact from attempted and
        // empty — and it is what stops a later sweep that adds the shopper
        // from looking like sellers suddenly started delivering.
        db.record_rungs_attempted(run_id, &[1]).await?;
        Some(db)
    };

    // ── The Bazaar, page by page ────────────────────────────────────────
    let mut listings: Vec<Listing> = Vec::new();
    let mut pages_read = 0u64;
    let mut catalog_complete = true;
    let mut reported_total: Option<u64> = None;
    let mut offset = 0u64;

    loop {
        if pages_read >= max_pages {
            tracing::warn!("stopping at the {max_pages}-page cap — the population is NOT complete");
            catalog_complete = false;
            break;
        }
        let url = format!(
            "https://api.cdp.coinbase.com/platform/v2/x402/discovery/resources\
             ?type=http&limit={BAZAAR_PAGE}&offset={offset}"
        );
        let fetched = fetcher.get(&url).await;

        let (page_listings, page_total, page_count) = match (&fetched.outcome, &fetched.body) {
            (Outcome::Fetched, Some(body)) => match bazaar::parse(body) {
                Ok(p) => {
                    tracing::info!(
                        "bazaar offset {offset}: {} items, {} listings, {} unreadable",
                        p.items_seen,
                        p.listings.len(),
                        p.items_unreadable
                    );
                    (p.listings, p.total, p.items_seen)
                }
                Err(e) => {
                    // The catalog served us something we do not understand.
                    // That is a fact about the catalog, and it stops this
                    // run from claiming a complete population.
                    tracing::error!("bazaar offset {offset}: unparseable ({e:?})");
                    catalog_complete = false;
                    (Vec::new(), None, 0)
                }
            },
            (outcome, _) => {
                tracing::error!(
                    "bazaar offset {offset}: {} — {}",
                    outcome.as_str(),
                    fetched.note.as_deref().unwrap_or("no detail")
                );
                catalog_complete = false;
                (Vec::new(), None, 0)
            }
        };

        if let Some(db) = &db {
            db.write_snapshot(
                run_id,
                bazaar::NAME,
                &fetched,
                i32::try_from(page_listings.len()).ok(),
            )
            .await?;
        }

        let listings_this_page = page_listings.len();
        listings.extend(page_listings);
        if let Some(t) = page_total {
            reported_total = Some(t);
        }
        pages_read += 1;

        // Stop when the catalog says we have seen everything, or when a page
        // carried nothing (which is how a catalog without a total ends).
        offset += BAZAAR_PAGE;
        let seen_everything = reported_total.is_some_and(|t| offset >= t);
        if !catalog_complete || seen_everything || (page_count == 0 && listings_this_page == 0) {
            break;
        }

        tokio::time::sleep(CATALOG_PAGE_DELAY).await;
    }

    // ── Scope first, THEN assemble ──────────────────────────────────────
    //
    // A third of the Bazaar's listings pay to Solana addresses. Read under
    // EVM rules they come out as `malformed_address` — a false claim about
    // somebody else's perfectly good listing. They are out of scope for
    // sweep 1 (§10.5), which is a fact about OUR coverage, and it is
    // reported as one.
    let (in_scope, out_of_scope) = catalog::partition_by_scope(&listings, &[network]);
    if !out_of_scope.is_empty() {
        let mut by_network: std::collections::BTreeMap<String, usize> = Default::default();
        for listing in &out_of_scope {
            *by_network
                .entry(sellers::network::canonical(&listing.network))
                .or_insert(0) += 1;
        }
        tracing::info!(
            "{} listings are on networks this sweep does not cover (not malformed — out of scope):",
            out_of_scope.len()
        );
        for (name, count) in &by_network {
            tracing::info!("  {name}: {count}");
        }
    }

    // ── The union, computed by the rules crate ──────────────────────────
    let scoped: Vec<Listing> = in_scope.into_iter().cloned().collect();
    let population = catalog::assemble(&scoped, Network::Evm);
    tracing::info!(
        "{} listings ({} in scope) → {} sellers ({} rejected)",
        listings.len(),
        scoped.len(),
        population.len(),
        population.rejected.len()
    );
    if !population.rejected.is_empty() {
        let mut by_reason: std::collections::BTreeMap<String, usize> = Default::default();
        for r in &population.rejected {
            *by_reason.entry(r.reason.clone()).or_insert(0) += 1;
        }
        for (reason, count) in &by_reason {
            tracing::warn!("  rejected {count} × {reason}");
        }
    }
    for (name, count) in catalog::coverage(&population) {
        tracing::info!("  {name}: {count} sellers");
    }
    if let Some(total) = reported_total {
        tracing::info!(
            "  the bazaar reported {total} resources; this run read {} pages",
            pages_read
        );
    }

    // The catalogs' price claims, kept for rung 7 (`consistent`) — the one
    // rung answered from evidence already held rather than a new request.
    let claims: Vec<sellers::consistent::Claim> = scoped
        .iter()
        .map(|l| sellers::consistent::Claim {
            resource: l.resource.clone(),
            pay_to: sellers::identity::normalize_pay_to(&l.pay_to, Network::Evm)
                .unwrap_or_else(|_| l.pay_to.clone()),
            network: l.network.clone(),
            amount: l.claimed_amount,
            asset: l.claimed_asset.clone(),
        })
        .collect();

    if let Some(db) = db {
        db.write_population(run_id, &population, &claims, started)
            .await?;
        let stored = db.seller_count(run_id).await?;
        // `finished` only if every catalog was read end to end. Anything
        // else is a smaller population, and a smaller population that reads
        // as complete is the worst row this table could hold.
        let status = if catalog_complete {
            "finished"
        } else {
            "failed"
        };
        db.close_run(run_id, status, i32::try_from(stored).unwrap_or(i32::MAX))
            .await?;
        tracing::info!("run {run_id} {status} — {stored} sellers stored");
        if !catalog_complete {
            anyhow::bail!("at least one catalog could not be read in full — run marked failed");
        }
    } else {
        tracing::info!("DRY RUN — nothing was written");
    }

    Ok(())
}

/// Unused today, and deliberately kept: the pause between pages is part of
/// the method, and a future catalog adapter must not have to rediscover it.
#[allow(dead_code)]
const _POLITENESS: Duration = CATALOG_PAGE_DELAY;
