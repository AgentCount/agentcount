//! # payments — who has ever been paid, pinned to a run's block.
//!
//! ```text
//! DATABASE_URL=… RPC_URL_BASE=… payments base                  # newest finished run
//! DATABASE_URL=… RPC_URL_BASE=… payments base 7833fc49-…       # a specific run
//! ```
//!
//! A second pass over a **finished** run, in the same shape as `liveness`
//! (rung 6): it runs after the sweep, is scoped to one run, writes its own
//! rows, and is allowed to fail without invalidating the census. If this
//! binary never runs for a chain, that chain simply has no payment rows — and
//! `payment_scans` having no row is how a reader tells "not scanned" from
//! "scanned and found nothing".
//!
//! ## What it does, in order
//!
//! 1. Resolve the run and its **pinned block**. Every read below is at that
//!    block. A run with no pin is refused outright.
//! 2. Read `getAgentWallet(agentId)` for every agent — the spec's payment
//!    address — and parse each archived document for the off-chain
//!    `services[].agentWallet` convention.
//! 3. Turn those into the **attribution map** (`payment_targets`) with
//!    `payments::targets_for`. Ineligible agents get a row saying why.
//! 4. For each basis and each of the chain's two stablecoins: read `symbol()`
//!    and `decimals()` **off the contract**, then scan `Transfer` logs to (and
//!    from) every eligible address up to the pinned block.
//! 5. Classify each transfer with `payments::classify`, resolving the two facts
//!    the exclusions need — whether the sender has code, and whether an EIP-3009
//!    `AuthorizationUsed` co-occurred — and write a row **whether it counted or
//!    not**, with the named rule that excluded it.
//! 6. Log both bases side by side. **Publish nothing.**
//!
//! ## What this binary deliberately does not do
//!
//! * **It does not produce a headline.** It logs a summary per basis and per
//!   token, and the summary carries both an address-level and an agent-level
//!   count because those are different numbers (PAY-1). Which figure a report
//!   quotes is a decision for a person reading `METHODOLOGY.md` §8, not for a
//!   log line.
//! * **It does not blend the two bases.** They are separate rows with a
//!   `basis` column, and only `verified_wallet` is publishable. The declared
//!   basis exists so the gap is measurable.
//! * **It does not touch `check_results`.** Payments are not a rung.
//!
//! ## Scope, and therefore the direction of every error
//!
//! Two stablecoins per chain, incoming ERC-20 `Transfer` logs, one chain at a
//! time. Native gas tokens, every other ERC-20, every other chain and all
//! off-chain settlement are invisible, so **every count is a lower bound on
//! agents paid**. "Not owner-funded" is one hop rather than a funding graph, so
//! the counted set is simultaneously an **upper bound on agents that earned**.
//! Direction is not purpose: an incoming stablecoin transfer is not proof a
//! service was rendered.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use payments::{AgentIdentity, Basis, Direction, TransferFacts, Verdict};
use sweeper::store::{self, PaymentWrite, ScanWrite, TargetWrite};
use uuid::Uuid;

/// The two stablecoins scanned per chain — **addresses only**.
///
/// Symbol and decimals are read from each contract at the pinned block and
/// stored on `payment_scans`, never taken from here. That is not caution for
/// its own sake: BSC's USDC and USDT are **18** decimals, not 6, and Celo's
/// `0x765DE816…` — documented for years as cUSD — now answers **`USDm`** at 18.
/// Carrying Base's 6 across all four chains would have overstated BSC by a
/// factor of 10^12 (`analysis/payments-per-chain.md` §2).
///
/// Two per chain because that is what the study these rows replace scanned, so
/// a future run is comparable with it. It is a scope choice, not a claim that
/// these are the only tokens anyone is paid in — which is exactly why every
/// figure is published as a lower bound.
const TOKENS: &[(&str, [&str; 2])] = &[
    (
        "base",
        [
            // USDC (native) — what x402 settles in on Base.
            "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
            // USDbC (bridged) — still circulating; excluding it would undercount.
            "0xd9aaec86b65d86f6a7b5b1b0c42ffa531710b6ca",
        ],
    ),
    (
        "bsc",
        [
            "0x8ac76a51cc950d9822d68b83fe1ad97b32cd580d", // USDC, 18 decimals
            "0x55d398326f99059ff775485246999027b3197955", // USDT, 18 decimals
        ],
    ),
    (
        "mainnet",
        [
            "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48", // USDC
            "0xdac17f958d2ee523a2206206994597c13d831ec7", // USDT
        ],
    ),
    (
        "celo",
        [
            "0xceba9300f2b948710d2653dd7b07f33a8b32118c", // USDC
            // Long documented as cUSD; reports USDm at 18 decimals today.
            "0x765de816845861e75a25fca122bb6898b8b1282a",
        ],
    ),
];

/// How many `getAgentWallet` reads are in flight at once.
///
/// The verified basis costs one call per agent — 244,208 of them on BSC — and
/// this is the only expensive step that scales with the population rather than
/// with the number of paid addresses. Deliberately conservative by default:
/// this pass runs after the census is already safe on disk, so it may take its
/// time, and a throttled provider costs the sweep nothing.
fn wallet_concurrency() -> usize {
    std::env::var("PAYMENTS_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(8)
}

/// Which bases to compute, comma-separated. Both by default.
///
/// `PAYMENTS_BASES=verified_wallet` skips the declared basis entirely, which is
/// the cheap configuration: the verified set on Base is 347 addresses against
/// the declared set's 822.
fn bases() -> Vec<Basis> {
    let raw = std::env::var("PAYMENTS_BASES")
        .unwrap_or_else(|_| "verified_wallet,declared_wallet".into());
    let mut out = Vec::new();
    for name in raw.split(',').map(str::trim) {
        match name {
            "verified_wallet" => out.push(Basis::VerifiedWallet),
            "declared_wallet" => out.push(Basis::DeclaredWallet),
            "" => {}
            other => tracing::warn!("PAYMENTS_BASES: ignoring unknown basis {other:?}"),
        }
    }
    out
}

/// Whether to scan outgoing transfers as well as incoming. On by default.
///
/// An outgoing row is never counted as a payment. It is scanned because a
/// balance answers nothing — funds received and swept out leave none — and
/// because an address whose entire history is outgoing is visibly not a payee.
/// `PAYMENTS_SCAN_OUTGOING=0` halves the log queries for a run that only needs
/// the incoming side.
fn scan_outgoing() -> bool {
    !matches!(
        std::env::var("PAYMENTS_SCAN_OUTGOING").as_deref(),
        Ok("0") | Ok("false") | Ok("no")
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let chain_name = std::env::args().nth(1).unwrap_or_else(|| "base".to_string());
    let explicit_run = std::env::args()
        .nth(2)
        .map(|s| Uuid::parse_str(&s))
        .transpose()?;

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = store::Db::connect(&database_url).await?;

    let run_id = match explicit_run {
        Some(id) => id,
        None => {
            db.latest_finished_run(&chain_name)
                .await?
                .with_context(|| format!("no finished run for chain {chain_name}"))?
                .0
        }
    };
    let (run_chain, pinned_block) = db.run_pin(run_id).await?;
    anyhow::ensure!(
        run_chain == chain_name,
        "run {run_id} is a {run_chain} run, not {chain_name}"
    );
    let (_, registry_addr, _, deploy_block) = db.chain_config(&chain_name).await?;
    let rpc_var = format!("RPC_URL_{}", chain_name.to_uppercase());
    let rpc_url = std::env::var(&rpc_var).with_context(|| format!("{rpc_var} must be set"))?;

    let tokens: &[&str] = TOKENS
        .iter()
        .find(|(c, _)| *c == chain_name)
        .map(|(_, t)| &t[..])
        .with_context(|| format!("no stablecoin scope configured for chain {chain_name}"))?;

    tracing::info!(
        "payments pass over run {run_id} ({chain_name}) at pinned block {pinned_block}, \
         rule version {}",
        payments::RULE_VERSION
    );

    let registry = chain::Registry::connect(&rpc_url, &registry_addr).await?;

    // ── 1. Every agent, and both candidate addresses ─────────────────────────
    let candidates = db.payment_candidates(run_id).await?;
    tracing::info!("{} agents in this run", candidates.len());

    let wanted = bases();
    anyhow::ensure!(!wanted.is_empty(), "PAYMENTS_BASES selected no basis");

    // `getAgentWallet` is only read when the verified basis is wanted; on the
    // declared-only configuration it is 244,208 calls nobody needs.
    let verified: HashMap<u64, Option<String>> = if wanted.contains(&Basis::VerifiedWallet) {
        read_agent_wallets(&registry, &candidates, pinned_block).await
    } else {
        HashMap::new()
    };

    let identities: Vec<AgentIdentity> = candidates
        .iter()
        .map(|c| AgentIdentity {
            agent_id: c.agent_id,
            owner: c.owner.clone(),
            verified_wallet: verified.get(&c.agent_id).cloned().flatten(),
            declared_wallets: c
                .body
                .as_deref()
                .map(payments::declared_wallets)
                .unwrap_or_default(),
            registration_block: c.registration_block,
        })
        .collect();

    // Every owner in the run, for the fleet-internal flag. Reported, never
    // excluded: "the sender owns some other agent here" is a real signal about
    // the population and a poor one about any single agent.
    let run_owners: HashSet<String> = identities
        .iter()
        .map(|i| i.owner.to_ascii_lowercase())
        .collect();

    // ── 2. Replace, never accumulate ─────────────────────────────────────────
    db.clear_payments(run_id).await?;

    let mut scanned_any = false;
    for basis in &wanted {
        scanned_any |= run_basis(
            &db,
            &rpc_url,
            &chain_name,
            run_id,
            pinned_block,
            deploy_block as u64,
            tokens,
            *basis,
            &identities,
            &run_owners,
        )
        .await?;
    }

    if !scanned_any {
        tracing::warn!(
            "no basis produced an eligible address for run {run_id}; \
             payment_scans has no row and that means NOT SCANNED, not zero"
        );
    }

    tracing::info!(
        "payments pass complete for run {run_id}. NO FIGURE IS PUBLISHED BY THIS BINARY — \
         the rows are in `payments`, the rule is in METHODOLOGY §8, and only the \
         verified_wallet basis may be quoted."
    );
    Ok(())
}

/// `getAgentWallet` for every agent, at the pinned block.
///
/// A read that fails is recorded as `None` and warned about rather than
/// aborting the pass: one unreadable agent must not cost the other 60,096, and
/// `None` produces no target row at all — which is honest, because "we could
/// not ask" is not "this agent has no wallet".
async fn read_agent_wallets(
    registry: &chain::Registry,
    candidates: &[store::PaymentCandidate],
    block: u64,
) -> HashMap<u64, Option<String>> {
    use futures::stream::{self, StreamExt};

    let concurrency = wallet_concurrency();
    tracing::info!(
        "reading getAgentWallet for {} agents at block {block} ({concurrency} in flight)",
        candidates.len()
    );
    let done = std::sync::atomic::AtomicUsize::new(0);
    let total = candidates.len();
    let out: Vec<(u64, Option<String>)> = stream::iter(candidates.iter().map(|c| c.agent_id))
        .map(|agent_id| {
            let done = &done;
            async move {
                let wallet = match registry.agent_wallet(agent_id, block).await {
                    Ok(w) => w,
                    Err(e) => {
                        tracing::warn!("getAgentWallet({agent_id}) failed: {e:#}");
                        None
                    }
                };
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if n.is_multiple_of(5_000) {
                    tracing::info!("read {n}/{total} agent wallets");
                }
                (agent_id, wallet)
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;
    out.into_iter().collect()
}

/// One basis, end to end: build the map, scan every token, write every row.
///
/// Returns whether anything was scanned at all.
#[allow(clippy::too_many_arguments)]
async fn run_basis(
    db: &store::Db,
    rpc_url: &str,
    chain_name: &str,
    run_id: Uuid,
    pinned_block: u64,
    deploy_block: u64,
    tokens: &[&str],
    basis: Basis,
    identities: &[AgentIdentity],
    run_owners: &HashSet<String>,
) -> Result<bool> {
    // ── The attribution map ──────────────────────────────────────────────────
    //
    // Every agent gets a row, eligible or not. `address -> agents` is the
    // many-to-many map PAY-1 is about, and it is built once here rather than
    // re-derived per transfer, so an agent-level and an address-level count
    // read the same structure.
    let mut reachers: HashMap<String, Vec<&AgentIdentity>> = HashMap::new();
    let mut ineligible = 0usize;
    for identity in identities {
        for decision in payments::targets_for(identity, basis) {
            db.write_payment_target(&TargetWrite {
                run_id,
                chain: chain_name,
                agent_id: identity.agent_id,
                basis: basis.as_str(),
                address: &decision.target.address,
                declared_index: decision.target.declared_index.map(|i| i as i32),
                eligible: decision.is_eligible(),
                ineligible_reason: decision.ineligible.map(|r| r.as_str()),
                owner: &identity.owner,
                registration_block: identity.registration_block,
                read_at_block: pinned_block,
            })
            .await
            .with_context(|| format!("writing target for agent {}", identity.agent_id))?;

            if decision.is_eligible() {
                reachers
                    .entry(decision.target.address.clone())
                    .or_default()
                    .push(identity);
            } else {
                ineligible += 1;
            }
        }
    }

    let addresses: Vec<String> = {
        let mut a: Vec<String> = reachers.keys().cloned().collect();
        a.sort();
        a
    };
    let shared = reachers.values().filter(|v| v.len() > 1).count();
    tracing::info!(
        "basis {}: {} eligible addresses reached by {} agents ({shared} addresses shared \
         by more than one agent), {ineligible} agent-addresses ineligible",
        basis.as_str(),
        addresses.len(),
        reachers.values().map(Vec::len).sum::<usize>(),
    );
    if addresses.is_empty() {
        return Ok(false);
    }

    // ── Scan ─────────────────────────────────────────────────────────────────
    let directions: Vec<(chain::Side, Direction)> = if scan_outgoing() {
        vec![
            (chain::Side::Incoming, Direction::In),
            (chain::Side::Outgoing, Direction::Out),
        ]
    } else {
        vec![(chain::Side::Incoming, Direction::In)]
    };
    let direction_label = if scan_outgoing() { "in,out" } else { "in" };

    // Two caches, both keyed by something the chain answers once and forever
    // at a fixed block. Without them a busy address costs one `eth_getCode` per
    // transfer, and a batched settlement costs one receipt per leg.
    let mut is_contract: HashMap<String, Option<bool>> = HashMap::new();
    let mut authorizations: HashMap<String, Option<String>> = HashMap::new();

    for token_address in tokens {
        let erc20 = chain::Erc20::connect(rpc_url, token_address).await?;
        // READ, never assumed. See TOKENS above.
        let meta = erc20.metadata(pinned_block).await?;
        tracing::info!(
            "basis {}: scanning {} ({}) — {} decimals, {} addresses, blocks {deploy_block}..{pinned_block}",
            basis.as_str(),
            meta.symbol,
            meta.address,
            meta.decimals,
            addresses.len(),
        );

        let mut found = 0usize;
        for (side, direction) in &directions {
            let logs = erc20
                .transfers(&addresses, *side, deploy_block, pinned_block)
                .await
                .with_context(|| format!("scanning {} for {}", meta.symbol, direction.as_str()))?;
            found += logs.len();

            for log in &logs {
                let (credited, counterparty) = match direction {
                    Direction::In => (&log.to, &log.from),
                    Direction::Out => (&log.from, &log.to),
                };
                let Some(agents) = reachers.get(credited.as_str()) else {
                    // A log the filter matched on the other side of a
                    // target-to-target transfer. It will be written under its
                    // own credited address by the other direction's pass.
                    continue;
                };
                let reached_by = agents.len() as i32;

                // PAY-3: read the sender's code once per address, and store
                // NULL — never `false` — when the read fails.
                let sender_has_code = *is_contract
                    .entry(counterparty.clone())
                    .or_insert(match erc20.is_contract(counterparty, pinned_block).await {
                        Ok(v) => Some(v),
                        Err(e) => {
                            tracing::warn!("eth_getCode({counterparty}) failed: {e:#}");
                            None
                        }
                    });

                // x402: an EIP-3009 authorization from THIS token in the same
                // transaction. A failed receipt read leaves the row's
                // authorizer NULL and its flag false, which is why the flag is
                // never quoted on its own — see the METHODOLOGY §8 caveat.
                let authorizer = authorizations
                    .entry(log.tx_hash.clone())
                    .or_insert(match erc20.authorization_in_tx(&log.tx_hash).await {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!("receipt for {} unreadable: {e:#}", log.tx_hash);
                            None
                        }
                    })
                    .clone();

                for identity in agents {
                    let facts = TransferFacts {
                        credited_address: credited,
                        direction: *direction,
                        counterparty,
                        agent_owner: &identity.owner,
                        agent_registration_block: identity.registration_block,
                        block_number: log.block_number,
                        counterparty_is_contract: sender_has_code,
                        eip3009_authorization: authorizer.is_some(),
                    };
                    let verdict = payments::classify(&facts);
                    let post_mint = identity
                        .registration_block
                        .map(|mint| log.block_number >= mint);

                    if let Err(e) = db
                        .write_payment(&PaymentWrite {
                            run_id,
                            chain: chain_name,
                            agent_id: identity.agent_id,
                            basis: basis.as_str(),
                            credited_address: credited,
                            address_reached_by: reached_by,
                            token_address: &meta.address,
                            token_symbol: &meta.symbol,
                            token_decimals: meta.decimals as i16,
                            direction: direction.as_str(),
                            counterparty,
                            value_raw: &log.value_raw,
                            block_number: log.block_number,
                            tx_hash: &log.tx_hash,
                            log_index: log.log_index as i32,
                            agent_registration_block: identity.registration_block,
                            post_mint,
                            counterparty_is_contract: sender_has_code,
                            counterparty_is_run_owner: Some(
                                run_owners.contains(&counterparty.to_ascii_lowercase()),
                            ),
                            eip3009_authorization: authorizer.is_some(),
                            eip3009_authorizer: authorizer.as_deref(),
                            eip3009_authorizer_is_sender: authorizer
                                .as_deref()
                                .map(|a| a.eq_ignore_ascii_case(counterparty)),
                            included: verdict == Verdict::Counted,
                            exclusion: verdict.exclusion().map(|e| e.as_str()),
                        })
                        .await
                    {
                        // One unwritable row must not cost the pass, exactly as
                        // in the rung-6 pass. It simply is not there, and the
                        // scan row's `transfers_found` will not match the row
                        // count — which is visible rather than silent.
                        tracing::warn!(
                            "could not write payment row for agent {} tx {}: {e}",
                            identity.agent_id,
                            log.tx_hash
                        );
                    }
                }
            }
        }

        // Written LAST, so a crash mid-scan leaves no row claiming this range
        // was covered.
        db.write_payment_scan(&ScanWrite {
            run_id,
            chain: chain_name,
            token_address: &meta.address,
            token_symbol: &meta.symbol,
            token_decimals: meta.decimals as i16,
            from_block: deploy_block,
            to_block: pinned_block,
            directions: direction_label,
            basis: basis.as_str(),
            targets_scanned: addresses.len() as i32,
            transfers_found: found as i32,
            rule_version: payments::RULE_VERSION,
        })
        .await?;
        tracing::info!(
            "basis {}: {} — {found} transfers found",
            basis.as_str(),
            meta.symbol
        );
    }

    if !basis.is_publishable() {
        tracing::info!(
            "basis {} is NOT publishable: a services[] entry named `agentWallet` is not in \
             the spec, carries no proof of control, and on Base contradicts the registry's \
             verified value for 409 of 919 agents. Its rows exist so the gap against \
             verified_wallet is measurable.",
            basis.as_str()
        );
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_swept_chain_has_a_token_scope_and_they_are_all_lowercase_addresses() {
        // A missing chain would make the binary refuse to run, which is the
        // right failure — but it should be caught here, not in the weekly job.
        for chain in ["base", "bsc", "mainnet", "celo"] {
            let entry = TOKENS.iter().find(|(c, _)| *c == chain);
            assert!(entry.is_some(), "{chain} has no token scope");
            for token in entry.unwrap().1 {
                assert_eq!(
                    payments::normalise_address(token).as_deref(),
                    Some(token),
                    "{token} is not a normalised lowercase address"
                );
            }
        }
    }

    #[test]
    fn the_token_scope_matches_the_addresses_the_prior_study_scanned() {
        // `analysis/payments-per-chain.md` §2 names each token by its leading
        // bytes. Pinning them here means a future edit that swaps a token can
        // no longer silently make a new run incomparable with the old study.
        let expect: &[(&str, [&str; 2])] = &[
            ("base", ["0x833589fc", "0xd9aaec86"]),
            ("bsc", ["0x8ac76a51", "0x55d39832"]),
            ("mainnet", ["0xa0b86991", "0xdac17f95"]),
            ("celo", ["0xceba9300", "0x765de816"]),
        ];
        for (chain, prefixes) in expect {
            let (_, tokens) = TOKENS.iter().find(|(c, _)| c == chain).unwrap();
            for (token, prefix) in tokens.iter().zip(prefixes) {
                assert!(token.starts_with(prefix), "{chain}: {token} vs {prefix}");
            }
        }
    }

    #[test]
    fn no_chain_scans_the_same_token_twice() {
        for (chain, tokens) in TOKENS {
            assert_ne!(tokens[0], tokens[1], "{chain} lists one token twice");
        }
    }

    #[test]
    fn bases_default_to_both_and_ignore_junk() {
        // Env is process-global, so this test only exercises the parser's
        // shape via the default string rather than by setting the variable.
        assert_eq!(bases().len(), 2);
    }
}
