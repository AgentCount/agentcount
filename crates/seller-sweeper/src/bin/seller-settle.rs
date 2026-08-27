//! # seller-settle — rung 6, from the chain rather than from the seller.
//!
//! ```text
//! RPC_URL_BASE=… DATABASE_URL=… seller-settle              # the latest sweep
//! RPC_URL_BASE=… DATABASE_URL=… seller-settle <run-id>
//! RPC_URL_BASE=… DATABASE_URL=… seller-settle --dry-run
//! SELLER_SETTLE_FROM_BLOCK=… …                             # widen the window
//! ```
//!
//! METHODOLOGY §10.3, rung 6 (`settled`). Every other rung asks a seller
//! something; this one asks the chain, so its evidence is checkable by
//! anyone with an RPC endpoint and does not require trusting this census at
//! all.
//!
//! ## Pinned, like every other on-chain measurement
//!
//! The scan runs to a block pinned once at the start and recorded on every
//! row. A rung 6 answer therefore means "as of block N", and re-running the
//! same scan against the same block reproduces it exactly — the property
//! `METHODOLOGY.md` §5 claims for the registration census, kept here.
//!
//! ## The window, stated rather than implied
//!
//! The scan starts at [`DEFAULT_FROM_BLOCK`] rather than at genesis. Base
//! has tens of millions of blocks and the x402 economy is younger than most
//! of them, so scanning from the beginning would cost enormously to learn
//! nothing about years when the protocol did not exist. The consequence is
//! stated rather than hidden: `fail` here means **no settlement in the
//! scanned window**, the window is on every row, and a reader can widen it
//! with `SELLER_SETTLE_FROM_BLOCK` and recompute.
//!
//! ## What it never counts
//!
//! Our own purchases. The shopper wallet is published in §10.4 before the
//! first purchase precisely so it can be excluded here; `sellers::settled`
//! holds that rule and this binary only passes it the address.

use std::collections::HashMap;

use anyhow::{Context, Result};
use chain::token::{Erc20, Side};
use seller_sweeper::store::Db;
use sellers::settled::{self, Settlement};
use uuid::Uuid;

/// Circle's canonical USDC on Base — the asset sweep 1 measures settlement
/// in, and the same contract `shop::SWEEP_ONE_ASSETS` prices against.
const USDC_BASE: &str = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";

/// How far back the scan looks, in blocks, when nothing overrides it.
///
/// 3,900,000 blocks is about 90 days at Base's two-second cadence. The
/// choice is a product one and is stated rather than implied: rung 6 asks
/// whether a payee is being paid NOW, not whether it ever was, and a
/// ninety-day window answers that for a weekly instrument. A reader who
/// wants "ever" can set `SELLER_SETTLE_FROM_BLOCK` and recompute — the
/// window is on every row precisely so that is possible.
const DEFAULT_LOOKBACK_BLOCKS: u64 = 3_900_000;

/// How many blocks one `eth_getLogs` request covers.
///
/// MEASURED against the production RPC (2026-08-21), not guessed. The
/// provider's rule is that a response is capped at 10,000 logs UNLESS the
/// range is 10,000 blocks or fewer, in which case there is no size limit.
/// So 10,000 is the only window size that cannot be refused, whatever the
/// payees do:
///
///   14,200,000 blocks, one busy payee ......... refused
///    2,000,000 blocks, one busy payee ......... refused
///      200,000 blocks, one busy payee ......... 2,937 logs, 5.4 s
///      200,000 blocks, 42 payees ................ refused
///       10,000 blocks, 1,000 payees ............. 1.5 s
///
/// Windowing PROACTIVELY at a size known to work matters more than it looks.
/// The shared scanner discovers the cap by failing and halving, so a
/// full-range request spends about as many refused requests as useful ones —
/// and reports nothing while it does. The first version of this binary ran
/// for twenty minutes in silence for exactly that reason.
const WINDOW_BLOCKS: u64 = 10_000;

/// How many payees go into one filter. The block range is what the response
/// cap binds on, not the address count, so a large batch is free — and it is
/// the difference between ~2,700 requests for a full sweep and ~200,000.
const PAYEES_PER_QUERY: usize = 1_000;

/// The shopper wallet from METHODOLOGY §10.4, published before the first
/// purchase so that this exclusion is checkable by anyone.
const SHOPPER_WALLET: &str = "0x8945b93E68C8927250DDFC41cd10EAc6CbEEd25f";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let explicit: Option<Uuid> = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(|a| Uuid::parse_str(a))
        .transpose()
        .context("run id must be a uuid")?;
    let explicit_from: Option<u64> = std::env::var("SELLER_SETTLE_FROM_BLOCK")
        .ok()
        .and_then(|v| v.parse().ok());

    let rpc_url = std::env::var("RPC_URL_BASE").context("RPC_URL_BASE must be set")?;
    let db_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = Db::connect(&db_url).await?;
    let run_id = match explicit {
        Some(id) => id,
        None => db
            .latest_run(sellers::network::BASE)
            .await?
            .context("no seller sweep to settle — run seller-crawl first")?,
    };

    let population = db.population_for_run(run_id).await?;
    anyhow::ensure!(!population.is_empty(), "run {run_id} has no sellers");

    let token = Erc20::connect(&rpc_url, USDC_BASE)
        .await
        .context("connecting to USDC on Base")?;
    // Pinned ONCE, before any log is read, and recorded on every row this
    // pass writes. A scan whose upper bound moved while it ran would produce
    // rows that cannot be reproduced together.
    let to_block = token.head_block().await.context("reading the head block")?;
    let from_block =
        explicit_from.unwrap_or_else(|| to_block.saturating_sub(DEFAULT_LOOKBACK_BLOCKS));
    anyhow::ensure!(
        to_block > from_block,
        "head block {to_block} is not after the window start {from_block}"
    );

    // Distinct payees: several sellers can share one payTo (§10.1 keeps them
    // separate as sellers, but the chain knows only the address), so the
    // chain is asked once per address and the answer is written to every
    // seller that address belongs to.
    // Only EVM payees can appear in an EVM token's transfer logs; asking the
    // chain about a base58 address would be a request with no possible hit.
    let mut payees: Vec<String> = population
        .iter()
        .filter(|s| sellers::identity::encoding_of(&s.pay_to) == sellers::identity::Network::Evm)
        .map(|s| s.pay_to.clone())
        .collect();
    payees.sort();
    payees.dedup();

    tracing::info!(
        "seller-settle {run_id}: {} sellers, {} distinct payees, USDC on Base, \
         blocks {from_block}..={to_block}{}",
        population.len(),
        payees.len(),
        if dry_run { " (DRY RUN)" } else { "" }
    );

    // Fixed windows, in order, with progress. The alternative — one request
    // for the whole range — is refused by the provider for any busy payee
    // and then halved until it fits, which spends about half its requests
    // learning a limit that is already known and reports nothing while it
    // does so.
    let mut transfers = Vec::new();
    let windows = to_block.saturating_sub(from_block).div_ceil(WINDOW_BLOCKS);
    let mut window_start = from_block;
    let mut done = 0u64;
    while window_start <= to_block {
        let window_end = (window_start + WINDOW_BLOCKS - 1).min(to_block);
        let found = token
            .transfers_batched(
                &payees,
                Side::Incoming,
                window_start,
                window_end,
                PAYEES_PER_QUERY,
            )
            .await
            .with_context(|| format!("scanning USDC transfers in {window_start}..={window_end}"))?;
        transfers.extend(found);
        done += 1;
        if done.is_multiple_of(50) || window_end == to_block {
            tracing::info!(
                "  {done}/{windows} windows, {} transfers so far",
                transfers.len()
            );
        }
        window_start = window_end + 1;
    }
    tracing::info!("{} incoming transfers found", transfers.len());

    // Group by payee. `transfers()` returns logs for the whole batch, and
    // the `to` field is what says whose they are.
    let mut by_payee: HashMap<String, Vec<Settlement>> = HashMap::new();
    for t in &transfers {
        by_payee
            .entry(t.to.to_ascii_lowercase())
            .or_default()
            .push(Settlement {
                from: t.from.clone(),
                block: t.block_number,
                tx_hash: t.tx_hash.clone(),
                value_raw: t.value_raw.clone(),
            });
    }

    let started = chrono::Utc::now();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut excluded_ours = 0usize;
    let mut excluded_self = 0usize;
    let mut out_of_scope = 0usize;

    for seller in &population {
        // A payee this scan CANNOT have found, because it does not settle on
        // the network being scanned, must not be recorded as unsettled. The
        // population spans every network (§10.5); this pass covers one, and
        // the difference between "we looked and found nothing" and "we did
        // not look here" is the whole reason `unprobed` exists.
        //
        // Recovered from the address's own shape, since a seller carries no
        // network: a non-EVM payee cannot appear in an EVM token's logs.
        if sellers::identity::encoding_of(&seller.pay_to) != sellers::identity::Network::Evm {
            out_of_scope += 1;
            if !dry_run {
                db.write_check(
                    run_id,
                    &seller.pay_to,
                    &seller.host,
                    6,
                    "settled",
                    "unprobed",
                    Some("out_of_scope_network"),
                    &serde_json::json!({
                        "scanned_network": sellers::network::BASE,
                        "scanned_token": USDC_BASE,
                        "note": "this payee does not settle on the scanned network",
                    }),
                    started,
                )
                .await?;
            }
            continue;
        }
        let found = by_payee
            .get(&seller.pay_to.to_ascii_lowercase())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let v = settled::judge(&seller.pay_to, found, SHOPPER_WALLET);
        match v.answer.status {
            sellers::SellerStatus::Pass => passed += 1,
            _ => failed += 1,
        }
        excluded_ours += v.excluded_ours;
        excluded_self += v.excluded_self;

        if dry_run {
            continue;
        }
        db.write_check(
            run_id,
            &seller.pay_to,
            &seller.host,
            6,
            "settled",
            v.answer.status.as_str(),
            v.answer.reason.as_deref(),
            // The window is part of the evidence, not context somebody has
            // to look up: a row that said "never paid" while meaning "not
            // paid since block N" would overstate what was established.
            &serde_json::json!({
                "token": USDC_BASE,
                "network": sellers::network::BASE,
                "scanned_from_block": from_block,
                "pinned_block": to_block,
                "settlements": v.settlements,
                "distinct_payers": v.distinct_payers,
                "first_block": v.first_block,
                "last_block": v.last_block,
                "excluded_ours": v.excluded_ours,
                "excluded_self": v.excluded_self,
                "shopper_wallet": SHOPPER_WALLET,
            }),
            started,
        )
        .await?;
    }

    if !dry_run {
        let mut attempted = db
            .run_meta(run_id)
            .await?
            .rungs_attempted
            .unwrap_or_default();
        if !attempted.contains(&6) {
            attempted.push(6);
            attempted.sort_unstable();
            db.record_rungs_attempted(run_id, &attempted).await?;
        }
    }

    tracing::info!("── rung 6, settled ──");
    tracing::info!("  pass: {passed}");
    tracing::info!("  fail (no settlement in window): {failed}");
    if out_of_scope > 0 {
        tracing::info!(
            "  unprobed (out_of_scope_network): {out_of_scope} — sellers that do not \
             settle on the scanned network, which is coverage this pass lacks and \
             never a statement about them"
        );
    }
    if excluded_ours > 0 || excluded_self > 0 {
        tracing::info!(
            "  excluded: {excluded_ours} from our own shopper wallet, \
             {excluded_self} self-transfers"
        );
    }
    if dry_run {
        tracing::info!("DRY RUN — nothing was written");
    }
    Ok(())
}
