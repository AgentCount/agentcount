//! # seller-probe — ask each seller the two questions one request answers.
//!
//! ```text
//! DATABASE_URL=… seller-probe                    # the latest run's population
//! DATABASE_URL=… seller-probe <run-id>           # a specific run
//! DATABASE_URL=… seller-probe --dry-run          # probe and report, write nothing
//! SELLER_PROBE_LIMIT=20 … seller-probe           # stop after N sellers (development)
//! ```
//!
//! Rungs 2 (`reachable`) and 3 (`quotes`), METHODOLOGY §10.3. A second pass
//! over a population `seller-crawl` already assembled — the same shape rung 6
//! has in the registration census, and for the same reason: the unit of work
//! is a HOST here, not a catalog, and per-host budgets need the whole
//! population in hand before the first request goes out.
//!
//! **One request per seller.** Reachability and the 402 handshake are the
//! same GET: asking twice would double this census's traffic to every seller
//! to learn nothing extra. What that one response means is
//! `sellers::reachable`'s decision, not this binary's.
//!
//! ## Politeness is the design, not a setting
//!
//! * robots.txt first, every time, no carve-out for the 402 handshake.
//! * **One request at a time per host**, with a pause between them. Sellers
//!   share hosts heavily — the first page of the Bazaar was twenty resources
//!   on one host — so a per-seller concurrency limit would be no limit at
//!   all for the hosts that matter.
//! * **At most 500 sellers probed per host per sweep** (§10.4). Sellers past
//!   that on a host are `unprobed` with reason `host_budget`: we chose not
//!   to ask, and the row says so rather than pretending the seller failed.
//! * Different hosts proceed in parallel, bounded, because they are
//!   different people's servers.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use futures::stream::StreamExt;
use seller_sweeper::fetcher::SellerProber;
use seller_sweeper::store::{Db, StoredSeller};
use sellers::consistent;
use sellers::identity::SellerId;
use sellers::reachable::{self, Observed};
use uuid::Uuid;

/// How many DISTINCT hosts may be in flight at once. Hosts are different
/// people's servers; within any one of them the pass is strictly sequential.
const HOST_CONCURRENCY: usize = 8;

/// METHODOLOGY §10.4: at most this many sellers probed per host per sweep.
const PER_HOST_BUDGET: usize = 500;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let explicit_run: Option<Uuid> = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(|a| Uuid::parse_str(a))
        .transpose()
        .context("run id must be a uuid")?;
    let limit: Option<usize> = std::env::var("SELLER_PROBE_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok());

    let url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = Db::connect(&url).await?;
    let run_id = match explicit_run {
        Some(id) => id,
        None => db
            .latest_run(sellers::network::BASE)
            .await?
            .context("no seller run to probe — run seller-crawl first")?,
    };

    let mut population = db.population_for_run(run_id).await?;
    if let Some(n) = limit {
        population.truncate(n);
    }
    anyhow::ensure!(!population.is_empty(), "run {run_id} has no sellers");

    // Group by host: the unit of politeness. BTreeMap so two passes visit
    // hosts in the same order.
    let mut by_host: BTreeMap<String, Vec<StoredSeller>> = BTreeMap::new();
    for seller in population {
        by_host.entry(seller.host.clone()).or_default().push(seller);
    }
    tracing::info!(
        "probing run {run_id}: {} sellers across {} hosts{}",
        by_host.values().map(Vec::len).sum::<usize>(),
        by_host.len(),
        if dry_run { " (DRY RUN)" } else { "" }
    );

    // This pass asks rungs 2, 3 and 7 — and NOT rung 4, which spends money
    // and runs only when the shopper does. Recorded rather than inferred, so
    // that a sweep which skipped a rung is legible as having skipped it.
    if !dry_run {
        let mut attempted = db
            .run_meta(run_id)
            .await?
            .rungs_attempted
            .unwrap_or_default();
        for rung in [2i16, 3, 7] {
            if !attempted.contains(&rung) {
                attempted.push(rung);
            }
        }
        attempted.sort_unstable();
        db.record_rungs_attempted(run_id, &attempted).await?;
    }

    let prober = SellerProber::new()?;
    let started = chrono::Utc::now();

    let results: Vec<HostOutcome> =
        futures::stream::iter(by_host.into_iter().map(|(host, sellers)| {
            let prober = &prober;
            let db = &db;
            async move {
                let mut outcome = HostOutcome::default();
                for (index, seller) in sellers.iter().enumerate() {
                    // The per-host budget. Beyond it we do not ask, and the
                    // row says we did not ask — never that the seller failed.
                    if index >= PER_HOST_BUDGET {
                        outcome.over_budget += 1;
                        if !dry_run {
                            for (rung, name) in [(2i16, "reachable"), (3, "quotes")] {
                                db.write_check(
                                    run_id,
                                    &seller.pay_to,
                                    &host,
                                    rung,
                                    name,
                                    "unprobed",
                                    Some("host_budget"),
                                    &serde_json::json!({
                                        "budget": PER_HOST_BUDGET,
                                        "host": host,
                                    }),
                                    started,
                                )
                                .await?;
                            }
                        }
                        continue;
                    }

                    let Some(resource) = seller.resources.first() else {
                        continue;
                    };
                    let observed = prober.probe(resource).await;
                    let id = SellerId {
                        pay_to: seller.pay_to.clone(),
                        host: host.clone(),
                    };
                    // The encoding this seller's payee was normalized under,
                    // recovered from the address itself — a seller carries no
                    // network, by design. A 402 from a Solana seller names a
                    // base58 payee, and reading it under EVM rules would
                    // report a working seller as quoting somebody else.
                    let encoding = sellers::identity::encoding_of(&seller.pay_to);
                    let verdict = reachable::judge(&id, &observed, encoding);

                    outcome.count(&verdict);
                    if !dry_run {
                        let evidence = serde_json::json!({
                            "resource": resource,
                            "observed": describe(&observed),
                        });
                        db.write_check(
                            run_id,
                            &seller.pay_to,
                            &host,
                            2,
                            "reachable",
                            verdict.reachable.status.as_str(),
                            verdict.reachable.reason.as_deref(),
                            &evidence,
                            started,
                        )
                        .await?;
                        let quote_evidence = serde_json::json!({
                            "resource": resource,
                            "requirements": verdict.requirements,
                        });
                        db.write_check(
                            run_id,
                            &seller.pay_to,
                            &host,
                            3,
                            "quotes",
                            verdict.quotes.status.as_str(),
                            verdict.quotes.reason.as_deref(),
                            &quote_evidence,
                            started,
                        )
                        .await?;

                        // Rung 7 needs no request of its own: both sides
                        // were already observed. The claim for THE PROBED
                        // resource is the one compared — a claim about a
                        // resource nobody asked about has no quote to
                        // disagree with.
                        let claims = db.claims_for(run_id, &seller.pay_to, &host).await?;
                        if let Some(claim) = claims.iter().find(|c| &c.resource == resource) {
                            let c = consistent::judge(claim, &verdict.requirements);
                            outcome.count_consistency(&c);
                            db.write_check(
                                run_id,
                                &seller.pay_to,
                                &host,
                                7,
                                "consistent",
                                c.answer.status.as_str(),
                                c.answer.reason.as_deref(),
                                &serde_json::json!({
                                    "resource": resource,
                                    "claimed": claim,
                                    "divergences": c.divergences,
                                }),
                                started,
                            )
                            .await?;
                        }
                    }
                }
                Ok::<HostOutcome, anyhow::Error>(outcome)
            }
        }))
        .buffer_unordered(HOST_CONCURRENCY)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;

    let total = results.iter().fold(HostOutcome::default(), |mut a, b| {
        a.merge(b);
        a
    });
    tracing::info!("── rung 2, reachable ──");
    for (status, n) in &total.reachable {
        tracing::info!("  {status}: {n}");
    }
    tracing::info!("── rung 3, quotes ──");
    for (status, n) in &total.quotes {
        tracing::info!("  {status}: {n}");
    }
    if total.over_budget > 0 {
        tracing::info!(
            "  {} sellers unprobed (host_budget — we chose not to ask)",
            total.over_budget
        );
    }
    for (reason, n) in &total.quote_reasons {
        tracing::info!("  quotes reason {reason}: {n}");
    }
    if !total.consistent.is_empty() {
        tracing::info!("── rung 7, consistent ──");
        for (status, n) in &total.consistent {
            tracing::info!("  {status}: {n}");
        }
        for (reason, n) in &total.divergences {
            tracing::info!("  diverged on {reason}: {n}");
        }
    }
    if dry_run {
        tracing::info!("DRY RUN — nothing was written");
    }
    Ok(())
}

/// A short, machine-readable note about what the probe saw — enough for the
/// evidence row to be checkable, never the seller's body itself.
fn describe(observed: &Observed) -> serde_json::Value {
    match observed {
        Observed::Response { status, body } => serde_json::json!({
            "kind": "response",
            "status": status,
            "body_bytes": body.as_ref().map(|b| b.len()),
        }),
        Observed::NotPermitted { reason } => {
            serde_json::json!({"kind": "not_permitted", "reason": reason})
        }
        Observed::ProbeFailed { reason } => {
            serde_json::json!({"kind": "probe_failed", "reason": reason})
        }
    }
}

/// Per-host tallies, merged into the run's report.
#[derive(Default)]
struct HostOutcome {
    reachable: BTreeMap<String, usize>,
    quotes: BTreeMap<String, usize>,
    quote_reasons: BTreeMap<String, usize>,
    consistent: BTreeMap<String, usize>,
    divergences: BTreeMap<String, usize>,
    over_budget: usize,
}

impl HostOutcome {
    fn count(&mut self, verdict: &reachable::ProbeVerdict) {
        *self
            .reachable
            .entry(verdict.reachable.status.as_str().to_string())
            .or_insert(0) += 1;
        *self
            .quotes
            .entry(verdict.quotes.status.as_str().to_string())
            .or_insert(0) += 1;
        if let Some(reason) = &verdict.quotes.reason {
            *self.quote_reasons.entry(reason.clone()).or_insert(0) += 1;
        }
    }

    fn count_consistency(&mut self, v: &consistent::ConsistencyVerdict) {
        *self
            .consistent
            .entry(v.answer.status.as_str().to_string())
            .or_insert(0) += 1;
        for d in &v.divergences {
            *self.divergences.entry(d.field().to_string()).or_insert(0) += 1;
        }
    }

    fn merge(&mut self, other: &HostOutcome) {
        for (k, v) in &other.reachable {
            *self.reachable.entry(k.clone()).or_insert(0) += v;
        }
        for (k, v) in &other.quotes {
            *self.quotes.entry(k.clone()).or_insert(0) += v;
        }
        for (k, v) in &other.quote_reasons {
            *self.quote_reasons.entry(k.clone()).or_insert(0) += v;
        }
        for (k, v) in &other.consistent {
            *self.consistent.entry(k.clone()).or_insert(0) += v;
        }
        for (k, v) in &other.divergences {
            *self.divergences.entry(k.clone()).or_insert(0) += v;
        }
        self.over_budget += other.over_budget;
    }
}
