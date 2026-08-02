//! # sweeper — run the conformance ladder over one chain, once.
//!
//! A run is the unit of work and the unit of citation: it pins a block, reads
//! every agent's current state, answers the rungs it can, and writes both the
//! database rows and the `data/<run_id>/` export. Runs are immutable; to get
//! newer answers you take a new run, never edit an old one. Resuming (see
//! [`sweep_resume`]) does not break that: it adds rows to a run that never
//! finished, it never edits a row already written.
//!
//! Day 2 wires in the probe layer: after each agent's chain snapshot, its
//! declared `tokenURI()` is fetched via `probe::Prober`, archived to
//! `http_archive`, and judged by rungs 2-5 (`resolvable`, `parseable`,
//! `conformant`, `bound`). Day 3 added rung 7 — then called `independent` —
//! constructed only for agents that had already passed rungs 1-5.
//!
//! **P0 FIX 4/5 (2026-07-29).** Rung 7 (renamed `attested`) is ungated here:
//! it is constructed for every agent that passes rung 1, full stop, never
//! conditioned on rungs 2 through 5. Reputation feedback lives in the
//! Reputation Registry, keyed by agent id, and is readable regardless of
//! whether that agent's document ever resolved, parsed, conformed, or
//! bound — there was never a real dependency on the document track, only an
//! accidental one from how the old gating happened to be written. See
//! `checks::ladder`'s module doc for how the ladder itself now encodes rung
//! 7 as its own independent track. The cost consequence is real and
//! intentional: a Reputation Registry read (`chain::Reputation::feedback`,
//! two RPC calls — `getClients` then, only if that returned addresses,
//! `getSummary` — never `getSummary` on an empty client list, which reverts
//! on this contract) now happens for essentially the whole population
//! (~60,000 agents, since rung 1 passes for nearly all of them) rather than
//! the ~1,437 that used to also pass rungs 2-5. Rung 6 is still ABSENT from
//! the output rather than reported as `skipped` — "we did not ask" and "we
//! could not ask" are different claims and the schema keeps them different.
//!
//! **P0 FIX 6 (2026-07-29).** Rungs 4 and 5, unlike rung 7 above, DO belong
//! to the Document track and DO depend on earlier rungs in it — but that
//! dependency is `run_ladder`'s to enforce, not this file's. Before this
//! fix, rungs 4 and 5 were only *constructed* when rung 3 produced a parsed
//! document, so an agent whose document never parsed got no rung-4/5 row at
//! all — absent, exactly like the unimplemented rung 6, rather than
//! `skipped`, which is what a lower-rung failure should produce. See
//! [`assemble_ladder`] for the fix and the full defect history.
//!
//! Two independent concurrency budgets drive the pipeline, on purpose (see
//! [`rpc_concurrency`] and [`fetch_concurrency`]): the RPC endpoint throttles
//! hard (a public free-tier provider), while HTTP fetches are limited
//! per-host by `probe` itself. Collapsing them into one shared number would
//! mean tuning one starves the other.

mod export;
// `store` moved into this crate's library so the `liveness` binary (rung 6)
// writes to the same tables through the same code — see `src/lib.rs`.
// `export` stays private here: only a full sweep produces `data/<run_id>/`.
use sweeper::store;

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures::stream::{self, StreamExt};
use uuid::Uuid;

/// How many `ownerOf`/`tokenURI` pairs to read at once. Conservative: a public
/// RPC endpoint is a shared resource and this is not a race. Lowered from 8
/// after Task 8's first live sweep hit Alchemy's free-tier "compute units per
/// second" cap immediately — override with `RPC_CONCURRENCY` without
/// recompiling.
const DEFAULT_RPC_CONCURRENCY: usize = 3;

fn rpc_concurrency() -> usize {
    std::env::var("RPC_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_RPC_CONCURRENCY)
}

/// How many `probe.fetch()` calls this stage keeps in flight at once. Kept
/// SEPARATE from [`rpc_concurrency`] — the RPC endpoint and the population of
/// HTTP hosts being fetched are different resources with different limits,
/// and collapsing them into one shared number would mean neither budget could
/// be tuned without affecting the other.
///
/// Reads the SAME `PROBE_CONCURRENCY` env var (and default) that
/// `probe::Prober` itself uses for its internal global semaphore — not a
/// second, independent knob — so this stage's own throttle can never be
/// tighter than the budget `Prober` was actually built with (which would
/// silently waste it) nor so loose that it stops being the number a reader
/// tuning `PROBE_CONCURRENCY` expects to be in effect.
fn fetch_concurrency() -> usize {
    std::env::var("PROBE_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(probe::DEFAULT_GLOBAL_CONCURRENCY)
}

/// How long a sweep may write nothing before the watchdog declares it dead.
///
/// Generous on purpose. A legitimately slow tail exists: `PER_HOST_CAP` is 2,
/// so a chain where one host holds thousands of agents crawls through them at
/// a couple of requests at a time — one observed sweep ran its tail at roughly
/// one agent per second because a single host held 974 agents. The timeout has
/// to sit well above that, and still well below "nobody notices until
/// tomorrow".
const DEFAULT_STALL_TIMEOUT_SECS: u64 = 900; // 15 minutes

fn stall_timeout_secs() -> u64 {
    std::env::var("SWEEP_STALL_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_STALL_TIMEOUT_SECS)
}

/// Watch a run's row count and kill the process if it stops moving.
///
/// **Why this is not just a log line.** The failure it exists for produced no
/// error, no log, and no CPU usage: an analysis scan reached 16,000 of 27,108
/// calls, the machine slept, its sockets died, and the process sat at 0% for
/// three hours looking exactly like slow progress. A sweep that hangs the same
/// way would hold a `running` row forever and quietly become a gap in the run
/// history — the one thing this project cannot afford, because the history is
/// the product.
///
/// It counts ROWS, not an in-process counter, deliberately: a sweep whose
/// consumer loop is alive but whose writes are failing is also stalled, and an
/// in-process counter would happily tick along while nothing landed.
///
/// On a stall it marks the run `stalled` with a reason and exits non-zero.
/// Exiting is the point — returning an error would be neater, but the main
/// task is by definition wedged on something that is not coming back, so
/// there is nobody left to return to.
fn spawn_stall_watchdog(db: store::Db, run_id: Uuid) {
    let timeout = std::time::Duration::from_secs(stall_timeout_secs());
    let poll = std::time::Duration::from_secs(30).min(timeout / 4);
    tokio::spawn(async move {
        let mut last_count: i64 = -1;
        let mut last_change = std::time::Instant::now();
        loop {
            tokio::time::sleep(poll).await;
            match db.swept_count(run_id).await {
                Ok(n) => {
                    if n != last_count {
                        last_count = n;
                        last_change = std::time::Instant::now();
                        continue;
                    }
                    let idle = last_change.elapsed();
                    if idle >= timeout {
                        let reason = format!(
                            "no agent written for {}s (stall timeout {}s); \
                             stopped at {n} agents",
                            idle.as_secs(),
                            timeout.as_secs()
                        );
                        tracing::error!(
                            "run {run_id} STALLED: {reason}. \
                             Marking the run stalled and exiting non-zero — a run that \
                             dies quietly becomes an invisible gap in the history."
                        );
                        if let Err(e) = db.fail_run(run_id, "stalled", &reason).await {
                            tracing::error!("could not even mark the run stalled: {e:#}");
                        }
                        std::process::exit(75); // EX_TEMPFAIL: retryable
                    }
                }
                // A database we cannot reach is not itself a stall — the sweep
                // may be fine and the watchdog blind. Say so and keep watching
                // rather than killing a healthy run.
                Err(e) => tracing::warn!("watchdog could not read progress: {e:#}"),
            }
        }
    });
}

/// The published contact string from `METHODOLOGY.md` (search
/// `agentcount-probe`) — the single source for the User-Agent's contact
/// portion. Declared here, not in `crates/probe`, and passed into
/// [`probe::Prober::new`] as a parameter, so the crate that actually sends
/// the header never hardcodes it and cannot drift from what METHODOLOGY.md
/// promises.
const PROBE_CONTACT_URL: &str = "https://agentcount.ai/methodology; contact: probes@agentcount.ai";

/// HTTPS gateways `ipfs://` URIs are tried against, in sequence, until one
/// answers 2xx or all are exhausted (P0 FIX 8 — reverses the earlier ruling
/// that used one disclosed gateway so a failure would be honestly
/// attributable; the owner confirmed the reversal, see
/// `CHANGELOG-METHODOLOGY.md`). Overridable via `IPFS_GATEWAYS`
/// (comma-separated, tried in the order given) for anyone who wants
/// different or self-hosted gateways; the evidence records every gateway
/// attempted and which one served each agent (`gateway_attempts`,
/// `via_gateway`) so a reader can tell an agent's failure from every
/// gateway's own.
fn ipfs_gateways() -> Vec<String> {
    std::env::var("IPFS_GATEWAYS")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|g| g.trim().to_string())
                .filter(|g| !g.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            vec![
                "https://ipfs.io/ipfs/".to_string(),
                "https://cloudflare-ipfs.com/ipfs/".to_string(),
                "https://gateway.pinata.cloud/ipfs/".to_string(),
            ]
        })
}

/// Reduce a `probe::FetchOutcome`'s raw scheme label to the six buckets
/// `checks::ResolvableInput` and the `http_archive.scheme` column agree on:
/// `"empty"`, `"unsupported"`, `"data"`, `"http"`, `"https"`, `"ipfs"`.
///
/// `FetchOutcome::scheme` alone is ambiguous for `data:` and `ipfs://`: a
/// MALFORMED one carries the SAME label as a genuine one (see
/// `probe::resolve::Target::Unsupported`'s doc comment — `probe` only knows
/// which scheme it tried to parse, not whether parsing succeeded), so
/// `scheme == "data"` alone cannot tell a decoded inline document from a
/// `data:` URI with no comma separator. `request_url` disambiguates: it is
/// set if, and only if, an actual HTTP(s) request was attempted — `fetch_http`
/// sets it as its very first action, before the netguard, robots check, or
/// the request itself can fail — so a malformed `ipfs://` (which never
/// reaches `fetch_http`) is caught here rather than misread as a passing
/// rung 2. A malformed `data:` URI is caught the same way via `body`: only a
/// successfully decoded inline payload ever has one.
///
/// **P0 FIX 7:** a `data:` URI declaring an unsupported `enc=` compression
/// algorithm also carries no `body` (there is nothing decoded to hand
/// forward) but DOES carry `.error` — that must still land in the `"data"`
/// bucket, not `"unsupported"`, so rung 2 can tell OUR limitation apart from
/// a malformed document (see `checks::resolvable`'s `"data"` match arm).
fn checks_scheme(outcome: &probe::FetchOutcome) -> String {
    if outcome.scheme.is_empty() {
        "empty".to_string()
    } else if outcome.request_url.is_some() {
        // A real HTTP(s) request was attempted (http, https, or ipfs via one
        // of the gateways) — keep whichever of those labels probe already
        // assigned.
        outcome.scheme.clone()
    } else if outcome.scheme == "data" && (outcome.body.is_some() || outcome.error.is_some()) {
        outcome.scheme.clone()
    } else {
        "unsupported".to_string()
    }
}

/// Sweep only the first N discovered agent ids, if set. Exists so a bounded
/// pilot run can validate the whole pipeline (DB rows, exports, rerun
/// command) before committing to a multi-hour full sweep of the real
/// population. When set, it MUST show up in the run's `rerun_command` —
/// a run that swept 2,000 of 59,998 agents but whose rerun command implies a
/// full sweep would misrepresent what was actually measured.
fn sweep_max_agents() -> Option<usize> {
    std::env::var("SWEEP_MAX_AGENTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
}

/// Resume an existing run instead of opening a new one. Set to a `run_id` a
/// previous sweep printed. Exists because a ~60,000-agent sweep runs for
/// hours, and a crash partway through — an RPC failure, a value the database
/// refuses (the NUL-byte hazard [`store::escape_nuls_for_postgres`-adjacent
/// code] guards against), a wedged connection, Ctrl-C — should not force
/// starting over from agent 0.
fn sweep_resume() -> Result<Option<Uuid>> {
    match std::env::var("SWEEP_RESUME") {
        Ok(s) => Ok(Some(Uuid::parse_str(&s).with_context(|| {
            format!("SWEEP_RESUME={s} is not a valid run id")
        })?)),
        Err(_) => Ok(None),
    }
}

/// Build rungs 4 and 5, hand every implemented rung to `run_ladder`, and
/// return what it decides.
///
/// **P0 FIX 6 (2026-07-29).** Before this fix, rungs 4 (`conformant`) and 5
/// (`bound`) were constructed only when `document` was `Some` — a document
/// that never parsed meant these two rungs were simply left out of the
/// vector handed to `run_ladder`, so `run_ladder` never got a row to mark
/// `Skipped` for them. The agent ended up with NO rung-4/5 row at all:
/// indistinguishable from rung 6, which genuinely is not implemented. In the
/// reference run this conflated "we did not ask" (`absent`, correct only for
/// rung 6) with "we could not ask, because rung 3 failed" (`skipped`, the
/// honest word) for 25,242 agents. `run_ladder` already knows how to tell
/// the two apart — see `checks::ladder`'s module doc — it simply never got
/// the chance, because these two rungs never reached it when there was
/// nothing to parse.
///
/// The fix: construct rung 4 and rung 5 UNCONDITIONALLY and always push them
/// into the vector `run_ladder` sees. When `document` is `None`,
/// `serde_json::Value::Null` stands in as the input document — and that
/// placeholder is guaranteed never to surface in the final result, because
/// rung 3 (the shared dependency both rungs sit above in the Document track)
/// is non-`Pass` in EVERY case where `document` is `None`: rung 3
/// (`checks::parseable`) only ever hands back `Some(document)` on its own
/// `Pass` path (see `rung3_parseable`'s return type). `run_ladder` therefore
/// always overwrites whatever `conformant`/`bound` computed from the
/// placeholder with `Skipped`, naming rung 3 as the blocker, before this
/// function returns. This is deliberately the ONLY place that decides
/// `Skipped` — `run_ladder` itself, never precomputed here — so the two
/// crates can't drift out of agreement about who owns skip-propagation (see
/// the ladder module doc's opening line).
///
/// Rung 6 (`live`) is still absent from the vector: it is not implemented
/// in this run, so "no row at all" remains the correct — and only —
/// signal for it. Rung 7 (`attested`) is passed through unchanged: it sits
/// on its own independent track (depends on rung 1 alone, see
/// `checks::ladder`), so a rung-2/3/4/5 failure must never touch it — that
/// is exercised directly below in `tests::a_rung_2_failure_never_touches_attested`.
#[allow(clippy::too_many_arguments)]
fn assemble_ladder(
    rung1: checks::CheckResult,
    rung2: checks::CheckResult,
    rung3: checks::CheckResult,
    document: Option<serde_json::Value>,
    spec_commit: &str,
    actual_agent_id: u64,
    actual_chain_id: u64,
    actual_registry: String,
    rung7: Option<checks::CheckResult>,
    now: DateTime<Utc>,
) -> Vec<checks::CheckResult> {
    // See the doc comment above: this placeholder is discarded by
    // `run_ladder` whenever it matters, because `document.is_none()`
    // implies rung 3 is non-`Pass`, which is exactly when `run_ladder`
    // overwrites rung 4 (and, transitively, rung 5) with `Skipped`.
    let document_for_ladder = document.unwrap_or(serde_json::Value::Null);
    let rung4 = checks::conformant(
        &checks::ConformantInput {
            document: document_for_ladder.clone(),
        },
        spec_commit,
        now,
    );
    let rung5 = checks::bound(
        &checks::BoundInput {
            document: document_for_ladder,
            actual_agent_id,
            actual_chain_id,
            actual_registry,
        },
        now,
    );

    let mut rungs = vec![rung1, rung2, rung3, rung4, rung5];
    if let Some(r7) = rung7 {
        rungs.push(r7);
    }
    checks::run_ladder(rungs)
}

/// Published as soon as a run row exists, so the error boundary in [`main`]
/// can mark that run `failed` without threading a handle back out of a
/// 700-line function.
///
/// A run left in `running` after the process is gone is the exact ambiguity
/// migration 0014 exists to remove: from the outside it is indistinguishable
/// from a sweep still in progress.
static CURRENT_RUN: std::sync::OnceLock<(store::Db, Uuid)> = std::sync::OnceLock::new();

#[tokio::main]
async fn main() -> Result<()> {
    let outcome = sweep().await;
    if let Err(e) = &outcome
        && let Some((db, run_id)) = CURRENT_RUN.get()
    {
        let reason = format!("{e:#}");
        tracing::error!("run {run_id} FAILED: {reason}");
        // Best effort: if the database is what failed, there is nowhere to
        // record that it failed. The non-zero exit and the log remain.
        if let Err(e2) = db.fail_run(*run_id, "failed", &reason).await {
            tracing::error!("could not mark run {run_id} failed: {e2:#}");
        }
    }
    outcome
}

async fn sweep() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let chain_arg = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "base".to_string());
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = store::Db::connect(&database_url).await?;

    // Resuming reloads chain, pinned_block, and every provenance column from
    // the EXISTING run, rather than deriving them fresh — see `sweep_resume`.
    let resume_run_id = sweep_resume()?;
    let resumed = match resume_run_id {
        Some(run_id) => {
            let r = db.load_run(run_id).await?;
            if r.chain != chain_arg {
                tracing::warn!(
                    "SWEEP_RESUME={run_id} was recorded for chain {}; ignoring the \
                     command-line chain argument {chain_arg:?}",
                    r.chain
                );
            }
            Some((run_id, r))
        }
        None => None,
    };
    let chain_name = resumed
        .as_ref()
        .map(|(_, r)| r.chain.clone())
        .unwrap_or(chain_arg);

    let rpc_var = format!("RPC_URL_{}", chain_name.to_uppercase());
    let rpc_url = std::env::var(&rpc_var).with_context(|| format!("{rpc_var} must be set"))?;
    let (chain_id, registry_addr, reputation_registry_addr, deploy_block) =
        db.chain_config(&chain_name).await?;
    let registry = chain::Registry::connect(&rpc_url, &registry_addr).await?;

    // Rung 7 needs a Reputation Registry client — but only if this chain
    // actually has one. `reputation_registry_addr` is `None` exactly when
    // `chains.reputation_registry` is NULL (e.g. no registry deployed yet on
    // some future chain); Base has one. Connecting is cheap (no RPC call
    // happens here, only `Registry::connect`-equivalent setup), so it is done
    // once up front, same as `registry` and `prober` above — never
    // reconnected per agent.
    let reputation = match reputation_registry_addr.as_deref() {
        Some(addr) => Some(chain::Reputation::connect(&rpc_url, addr).await?),
        None => None,
    };

    // One shared prober for the whole run. `chain_id`/`registry_addr` above
    // are the SAME values rung 5 compares each document's declared binding
    // against below — a single source so rung 1's provenance and rung 5's
    // "reality" can never quietly disagree.
    let gateways = ipfs_gateways();
    let prober = probe::Prober::new(PROBE_CONTACT_URL, &gateways)?;

    // `deploy_block` is not used for ENUMERATION — agent ids are found by
    // binary search on `ownerOf` existence, not by scanning logs, because a
    // log scan cannot see an agent whose registration event it missed. It is
    // used below as the lower bound of the `Registered` scan that captures the
    // minter, where a missed event costs one null field rather than a missing
    // agent.
    let registration_from_block = deploy_block.max(0) as u64;

    let (
        run_id,
        pinned,
        schema_version,
        checker_version,
        checker_commit,
        spec_commit,
        rerun,
        started_at,
        already_swept,
    ) = match resumed {
        Some((run_id, r)) => {
            let already_swept = db.swept_agent_ids(run_id, &chain_name).await?;
            tracing::info!(
                "resuming run {run_id} on {chain_name} at pinned block {} — \
                     {} agent(s) already swept, resuming the remainder",
                r.pinned_block,
                already_swept.len()
            );
            (
                run_id,
                r.pinned_block,
                r.schema_version,
                r.checker_version,
                r.checker_commit,
                r.spec_commit,
                r.rerun_command,
                r.started_at.to_rfc3339(),
                already_swept,
            )
        }
        None => {
            let pinned = registry.pinned_block().await?;
            tracing::info!("sweeping {chain_name} at block {pinned}");

            let run_id = Uuid::new_v4();
            let checker_commit = env!("CHECKER_COMMIT").to_string();
            let max_agents = sweep_max_agents();
            // The rerun command must describe what THIS run actually
            // swept. A pilot capped by SWEEP_MAX_AGENTS is not reproduced
            // by the bare command below — omitting the cap here would
            // make the archived run claim a full sweep it never did.
            let rerun = match max_agents {
                Some(n) => format!(
                    "SWEEP_MAX_AGENTS={n} cargo run -p sweeper -- {chain_name}   # at block {pinned}"
                ),
                None => format!("cargo run -p sweeper -- {chain_name}   # at block {pinned}"),
            };

            db.open_run(&store::RunMeta {
                run_id,
                chain: chain_name.clone(),
                pinned_block: pinned,
                schema_version: checks::SCHEMA_VERSION,
                checker_version: checks::CHECKER_VERSION.to_string(),
                checker_commit: checker_commit.clone(),
                spec_commit: checks::SPEC_COMMIT.to_string(),
                rerun_command: rerun.clone(),
            })
            .await?;

            (
                run_id,
                pinned,
                checks::SCHEMA_VERSION,
                checks::CHECKER_VERSION.to_string(),
                checker_commit,
                checks::SPEC_COMMIT.to_string(),
                rerun,
                Utc::now().to_rfc3339(),
                HashSet::new(),
            )
        }
    };
    let checker_commit = checker_commit.as_str();
    let checker_version = checker_version.as_str();
    let spec_commit = spec_commit.as_str();

    let max_agents = sweep_max_agents();
    // Enumerated at the PINNED block (the original one, if resuming) so the
    // population matches what the first session saw, not whatever exists on
    // chain right now.
    // Armed before any long-running work: from here on a hang is detected and
    // reported rather than sat through, and an error ends with the run marked
    // `failed` rather than left looking like it is still going.
    let _ = CURRENT_RUN.set((db.clone(), run_id));
    spawn_stall_watchdog(db.clone(), run_id);

    let mut ids = registry.enumerate_agent_ids(pinned).await?;
    let discovered = ids.len();
    if let Some(n) = max_agents {
        ids.truncate(n);
    }
    // `planned` is this run's TOTAL intended scope — cumulative across every
    // session that has worked on it, not just this one. It equals
    // `already_swept.len() + ids.len()` below by construction (the same list
    // just gets filtered), which is what keeps the swept/unreadable math at
    // the end honest without having to remember a prior session's counts.
    let planned = ids.len();
    ids.retain(|id| !already_swept.contains(id));
    let remaining = ids.len();
    tracing::info!(
        "{discovered} agent ids discovered; {planned} in scope for this run \
         ({} already swept, {remaining} remaining this session){}",
        already_swept.len(),
        max_agents
            .map(|n| format!(" (SWEEP_MAX_AGENTS={n})"))
            .unwrap_or_default()
    );

    // Read current state for each id, bounded. `buffer_unordered` keeps at most
    // RPC_CONCURRENCY reads in flight; results arrive out of order, which is
    // fine because each carries its own agent_id.
    // The manifest is written BEFORE the sweep, so a run that dies partway
    // still leaves a readable, self-describing directory on disk — and then
    // REWRITTEN at the end with what actually happened. Writing it only once,
    // up front, would mean the artefact a reader downloads reports the
    // population we intended to sweep while the files beside it hold however
    // many we managed: the incompleteness would be discoverable only by
    // counting rows, which is exactly what this project promises never to
    // make someone do.
    let manifest = |swept: Option<usize>,
                    unreadable: Option<usize>,
                    unwritable: Option<usize>,
                    finished: Option<String>| {
        export::RunManifest {
            run_id: run_id.to_string(),
            chain: &chain_name,
            chain_id: chain_id as u64,
            registry: &registry_addr,
            pinned_block: pinned,
            started_at: started_at.clone(),
            schema_version,
            checker_version,
            checker_commit,
            spec_commit,
            rerun_command: &rerun,
            // The writing binary's own identity — NOT the local
            // `checker_commit`, which on a resume is the sweep-time value
            // read back from the database and may name a different build
            // than the one writing this file.
            exporter_version: env!("CARGO_PKG_VERSION"),
            exporter_commit: env!("CHECKER_COMMIT"),
            agent_count: planned,
            swept,
            unreadable,
            unwritable,
            finished_at: finished,
        }
    };
    export::write_manifest(&manifest(None, None, None, None))?;

    // Persist each agent AS IT ARRIVES rather than collecting the whole
    // population first. At 60,000 agents a sweep runs for hours, and a
    // collect-then-write shape means a crash, a dropped connection, or a
    // Ctrl-C at hour three discards every read — plus the database shows
    // nothing until the very end, so there is no way to tell a working sweep
    // from a wedged one.
    // Two chained stages, each with its OWN `buffer_unordered` — and
    // therefore its own concurrency budget — rather than one pipeline shared
    // end to end. Stage 1 reads the chain (bounded by `rpc_concurrency`);
    // stage 2 fetches the agent's declared document over HTTP (bounded by
    // `fetch_concurrency`, independent of stage 1 and matched to
    // `probe::Prober`'s own internal global cap — see that function's doc).
    // An RPC failure is carried through stage 2 as `Err` rather than
    // filtered out beforehand: filtering here would need a shared mutable
    // counter reached from inside the stream combinators, and threading the
    // failure through as data is simpler and cannot lose the error message.
    // ── Minter capture (schema 6) ────────────────────────────────────────────
    //
    // Who sent the registration transaction. Not the same role as `owner` — a
    // platform minting on a customer's behalf is the ordinary case, and on one
    // chain two addresses minted 87.9% of the population.
    //
    // Done as a pre-pass, not a fourth pipeline stage, because it is one
    // chain-wide log scan rather than a per-agent read. The expensive half is
    // resolving each transaction's sender, and that is bounded by the number of
    // DISTINCT transactions, not agents: batch minters register many agents per
    // transaction, so this is usually far cheaper than one call per agent.
    //
    // Failure here is never fatal. A missing minter is a null column; it must
    // not cost the run, because the census's job is the ladder and this is
    // provenance alongside it.
    let registrations = match registry
        .registrations(registration_from_block, pinned)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "registration scan failed ({e:#}); minter will be null for this run \
                 — the ladder is unaffected"
            );
            Default::default()
        }
    };
    let mut minters: HashMap<String, Option<String>> = HashMap::new();
    if !registrations.is_empty() {
        let distinct_txs: Vec<String> = registrations
            .values()
            .map(|r| r.tx_hash.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        tracing::info!(
            "registration scan: {} agents across {} distinct transactions",
            registrations.len(),
            distinct_txs.len()
        );
        let resolved: Vec<(String, Option<String>)> = stream::iter(distinct_txs)
            .map(|tx| {
                let registry = &registry;
                async move {
                    let sender = registry.tx_sender(&tx).await.unwrap_or_else(|e| {
                        tracing::warn!("tx_sender({tx}) failed: {e:#}");
                        None
                    });
                    (tx, sender)
                }
            })
            .buffer_unordered(rpc_concurrency())
            .collect()
            .await;
        minters.extend(resolved);
        tracing::info!(
            "minters resolved for {}/{} transactions",
            minters.values().filter(|m| m.is_some()).count(),
            minters.len()
        );
    }
    let registrations = &registrations;
    let minters = &minters;

    let mut stream = stream::iter(ids)
        .map(|id| {
            let registry = &registry;
            async move { (id, registry.snapshot(id, pinned).await) }
        })
        .buffer_unordered(rpc_concurrency())
        .map(|(id, result)| {
            let prober = &prober;
            async move {
                match result {
                    Ok(s) => {
                        let outcome = prober.fetch(&s.agent_uri).await;
                        (id, Ok((s, outcome)))
                    }
                    Err(e) => (id, Err(e)),
                }
            }
        })
        .buffer_unordered(fetch_concurrency())
        // Stage 3: the Reputation Registry read for rung 7.
        //
        // This used to happen in the consumer loop below, one agent at a time,
        // and it was the ceiling on the entire sweep. The two stages above are
        // concurrent, but the loop that drains them is not — so a serial
        // network round trip per agent capped throughput at 1/latency
        // (measured: ~3.2 agents/sec on Celo) no matter what `RPC_CONCURRENCY`
        // or `PROBE_CONCURRENCY` were set to. Raising either changed nothing,
        // because neither governed the serial tail.
        //
        // Moving only the READ here leaves every verdict where it was: the
        // loop still builds rung 7 from `checks::attested`, with the same
        // `now` it uses for every other rung, so no evidence and no timestamp
        // changes shape. This stage fetches; it does not judge.
        //
        // The rung-1 gate is preserved rather than dropped — P0 FIX 4/5 spends
        // no RPC call on an agent whose rung 1 failed, and that frugality is
        // deliberate. It is preserved by CALLING `checks::registered`, never by
        // re-deriving its rule here: a second copy of "what makes rung 1 pass"
        // is exactly the drift this project refuses. The duplicate call is pure
        // and does no I/O, and its status cannot differ from the loop's —
        // `registered` decides on the owner address alone, and takes `now` only
        // to stamp `checked_at`.
        .map(|(id, result)| {
            let reputation = &reputation;
            let registry_addr = &registry_addr;
            async move {
                match result {
                    Ok((s, outcome)) => {
                        let gated_in = checks::registered(
                            &checks::RegisteredInput {
                                chain_id: chain_id as u64,
                                registry: registry_addr.clone(),
                                token_id: s.token_id.to_string(),
                                owner: s.owner.clone(),
                                block_number: s.block_number,
                                tx_hash: None,
                            },
                            Utc::now(),
                        )
                        .status
                            == checks::CheckStatus::Pass;

                        // `None` means no read was attempted — either rung 1
                        // did not pass, or this chain has no Reputation
                        // Registry. The loop tells those two apart itself and
                        // never needs this to say which.
                        let feedback = match (gated_in, reputation.as_ref()) {
                            (true, Some(rep)) => Some(rep.feedback(s.agent_id, pinned).await),
                            _ => None,
                        };
                        (id, Ok((s, outcome, feedback)))
                    }
                    Err(e) => (id, Err(e)),
                }
            }
        })
        .buffer_unordered(rpc_concurrency());

    // Session-local, but see the `planned` comment above: because `ids` here
    // is exactly `planned` minus `already_swept`, every id in it is attempted
    // exactly once (success or failure), so `already_swept.len() + swept +
    // unreadable + unwritable == planned` holds whether this is a fresh run
    // (already_swept empty) or a resumed one — no need to have persisted a
    // prior session's failure count anywhere to report the true cumulative
    // totals below.
    let mut swept = 0usize;
    let mut unreadable = 0usize;
    // Read fine, but the per-agent database TRANSACTION never committed —
    // either a permanent error (bad data, a constraint violation) or a
    // transient one that never succeeded within `store::retry_transient`'s
    // budget. Counted and reported the same way `unreadable` is: absent from
    // this run, never recorded as a `fail`, never a reason to abort the
    // remaining agents. See the write site below for why `swept` is NOT
    // incremented for these — the transaction rolled back, so nothing about
    // this agent was actually persisted.
    let mut unwritable = 0usize;

    while let Some((id, result)) = stream.next().await {
        let (s, outcome, feedback) = match result {
            Ok(pair) => pair,
            Err(e) => {
                // An RPC failure is OUR problem, not the agent's: leave the
                // agent out of this run rather than recording a `fail` about
                // them. The count is reported at the end so the omission is
                // visible instead of silent.
                tracing::warn!("snapshot({id}) failed: {e:#}");
                unreadable += 1;
                continue;
            }
        };
        // Attach the registration provenance captured in the pre-pass. Absent
        // is absent: an agent whose `Registered` event we did not see keeps
        // three nulls rather than an invented value.
        let mut s = s;
        if let Some(reg) = registrations.get(&id) {
            s.registration_tx_hash = Some(reg.tx_hash.clone());
            s.registration_block = Some(reg.block_number);
            s.minter = minters.get(&reg.tx_hash).cloned().flatten();
        }
        let s = &s;
        let now = Utc::now();

        // The scheme bucket every downstream rung and the archive row agree
        // on — see `checks_scheme`'s doc comment for why this can't just be
        // `outcome.scheme` verbatim.
        let scheme = checks_scheme(&outcome);

        let rung1 = checks::registered(
            &checks::RegisteredInput {
                chain_id: chain_id as u64,
                registry: registry_addr.clone(),
                token_id: s.token_id.to_string(),
                owner: s.owner.clone(),
                block_number: s.block_number,
                // Captured from the `Registered` scan above (schema 6). Still
                // null, never invented, when that scan did not see this agent.
                tx_hash: s.registration_tx_hash.clone(),
            },
            now,
        );

        let inline_bytes = if scheme == "data" {
            outcome.body.as_ref().map(Vec::len)
        } else {
            None
        };
        // The URI goes into rung 2's evidence, which is a `jsonb` column, and
        // Postgres rejects `\0` in jsonb outright — so the same escape the
        // TEXT writes use has to be applied here too, or an agent with a NUL
        // in its tokenURI aborts the run AFTER its snapshot row has landed.
        // Escaping once here means the store and the checks cannot disagree
        // about what the URI was.
        let uri_for_evidence =
            store::escape_nuls_for_postgres(s.agent_id, &s.agent_uri).into_owned();
        let rung2 = checks::resolvable(
            &checks::ResolvableInput {
                uri: uri_for_evidence,
                scheme: scheme.clone(),
                request_url: outcome.request_url.clone(),
                final_url: outcome.final_url.clone(),
                http_status: outcome.http_status,
                elapsed_ms: outcome.elapsed_ms,
                error: outcome.error.clone(),
                inline_bytes,
                via_gateway: outcome.via_gateway.clone(),
                inline_decode_variant: outcome
                    .inline_decode
                    .as_ref()
                    .map(|d| d.variant.to_string()),
                inline_decode_algorithm: outcome
                    .inline_decode
                    .as_ref()
                    .and_then(|d| d.algorithm.clone()),
                gateway_attempts: if outcome.gateway_attempts.is_empty() {
                    None
                } else {
                    serde_json::to_value(&outcome.gateway_attempts).ok()
                },
            },
            now,
        );

        let (rung3, document) = checks::parseable(
            &checks::ParseableInput {
                body: outcome.body.clone(),
                content_type: outcome.content_type.clone(),
                body_sha256: outcome.body_sha256.clone(),
                truncated: outcome.truncated,
            },
            now,
        );

        // Same jsonb hazard, one layer deeper. A document may legally contain
        // ` ` inside a string, which serde_json parses into a real NUL —
        // and rungs 4 and 5 copy document-derived values (field names,
        // `declared_registry`) straight into their evidence. Escaping the
        // parsed document once here keeps every downstream evidence object
        // insertable. Field PRESENCE is unaffected: the escape only rewrites
        // string contents, so a key named `name` is still named `name`.
        let document = document.map(|mut d| {
            store::escape_nuls_in_json(&mut d);
            d
        });

        // P0 FIX 4/5: rung 7 (`attested`) is gated on rung 1 ALONE — never on
        // rungs 2 through 5. `document` (possibly `None`) is passed into
        // `assemble_ladder` below, which is where rungs 4 and 5 actually get
        // built — see P0 FIX 6.
        let reaches_attested = rung1.status == checks::CheckStatus::Pass;

        // `None` here means one of two different things, and the branches
        // below keep them distinct:
        //   - rung 1 didn't pass (an owner-is-zero-address token; vanishingly
        //     rare in practice) → rung 7 is simply not asked, and
        //     `run_ladder` would mark it `Skipped` if it were present — but
        //     since we choose not to spend the RPC call here, it is left out
        //     of `rungs` entirely, same as the unimplemented rung 6.
        //   - rung 1 DID pass but the feedback read itself failed → that's
        //     OUR problem, not the agent's, so — same as an unreadable
        //     snapshot above — this agent is left out of the run entirely
        //     (`continue`) rather than recording anything false about it.
        let rung7 = if !reaches_attested {
            None
        } else if reputation.is_some() {
            // Already read, concurrently, by stage 3 above — the verdict is
            // still built here, from `checks::attested`, with this agent's
            // `now`.
            match feedback {
                Some(Ok(fr)) => Some(checks::attested(
                    &checks::AttestedInput {
                        clients: fr.clients,
                        feedback_count: fr.feedback_count,
                        registry_available: true,
                    },
                    now,
                )),
                Some(Err(e)) => {
                    tracing::warn!(
                        "agent {}: reputation feedback read failed: {e:#} — leaving this \
                         agent out of the run rather than recording anything false about it",
                        s.agent_id
                    );
                    unreadable += 1;
                    continue;
                }
                // Unreachable: stage 3 gates on the same `checks::registered`
                // call this branch's `reaches_attested` came from, and that
                // status depends on the owner address alone. Handled as a
                // failed read rather than asserted away — if the two ever did
                // disagree, leaving the agent out is the honest outcome, and
                // inventing a rung-7 status for it is not.
                None => {
                    tracing::error!(
                        "agent {}: rung 1 passed but no reputation read was attempted — \
                         the stage gate and the loop gate disagreed, which should be \
                         impossible; leaving this agent out of the run",
                        s.agent_id
                    );
                    unreadable += 1;
                    continue;
                }
            }
        } else {
            // `chains.reputation_registry` is NULL for this chain: we cannot
            // check, which is our limitation, not the agent's — `Error`,
            // never `Fail`. No RPC call is made; `checks::attested` alone
            // decides the status from `registry_available: false`.
            Some(checks::attested(
                &checks::AttestedInput {
                    clients: Vec::new(),
                    feedback_count: 0,
                    registry_available: false,
                },
                now,
            ))
        };

        // P0 FIX 6: rungs 4 and 5 are ALWAYS constructed here (never
        // conditioned on `document.is_some()`) and always handed to
        // `run_ladder` — see `assemble_ladder`'s doc comment for the full
        // defect history and why the `document.is_none()` placeholder can
        // never leak into the final result.
        let results = assemble_ladder(
            rung1,
            rung2,
            rung3,
            document,
            spec_commit,
            s.agent_id,
            chain_id as u64,
            registry_addr.clone(),
            rung7,
            now,
        );

        // All three writes — snapshot, archive, check results — land in ONE
        // transaction (see `store::Db::write_agent`), retried with bounded
        // backoff only while `store::classify_error` says the failure is
        // transient. A permanent error (bad data, a constraint violation)
        // comes back on the first attempt.
        //
        // Deliberately NOT `?` here: propagating would abort the entire
        // multi-hour run over one agent, which is exactly the failure mode
        // that has already cost two restarts. Instead: roll back (automatic
        // — the transaction was never committed), log loudly with the agent
        // id and SQLSTATE, count the agent unwritable, and move on to the
        // next one. `swept` is NOT incremented below for this agent: the
        // transaction rolled back, so nothing about it was actually
        // persisted, and `runs.agent_count` must keep meaning "agents
        // actually written," not "agents attempted."
        // Named (not a literal built inline in the closure below): the
        // closure is called more than once on retry, and each call must
        // borrow the SAME long-lived value across its `.await` rather than a
        // fresh temporary that would be dropped as soon as the closure
        // expression finished evaluating.
        let write = store::AgentWrite {
            run_id,
            chain: &chain_name,
            snapshot: s,
            requested_uri: &s.agent_uri,
            scheme: &scheme,
            outcome: &outcome,
            results: &results,
        };
        let write_result = store::retry_transient(|| db.write_agent(&write)).await;
        if let Err(e) = write_result {
            let sqlstate = e
                .as_database_error()
                .and_then(|d| d.code())
                .map(|c| c.into_owned());
            // Either classification ends up here: a Permanent error returned
            // on its first attempt, or a Transient one that never succeeded
            // within `retry_transient`'s budget. Both mean the SAME thing to
            // the run: this agent's transaction never committed.
            let why = match store::classify_error(&e) {
                store::Classification::Permanent => "permanent error",
                store::Classification::Transient => "transient error, retries exhausted",
            };
            tracing::error!(
                "agent {}: database write did not succeed ({why}, sqlstate={sqlstate:?}): {e:#} \
                 — rolled back, counting unwritable, continuing to the next agent",
                s.agent_id
            );
            unwritable += 1;
            continue;
        }

        // The export file is written ONLY after the transaction above
        // committed. A filesystem write cannot join a database transaction,
        // so this ordering — never writing the file first — is what
        // guarantees no orphan JSON file can exist for an agent the database
        // rejected.
        export::write_agent(&export::AgentDocument {
            run_id: run_id.to_string(),
            chain: &chain_name,
            agent_id: s.agent_id,
            token_id: s.token_id.to_string(),
            owner: &s.owner,
            agent_uri: &s.agent_uri,
            block_number: s.block_number,
            checks: &results,
            checker_commit,
            spec_commit,
            http_status: outcome.http_status,
            content_type: outcome.content_type.as_deref(),
            body_bytes: outcome.body.as_ref().map(Vec::len),
            body_sha256: outcome.body_sha256.as_deref(),
            final_url: outcome.final_url.as_deref(),
        })?;

        swept += 1;
        // Heartbeat, batched so it costs one UPDATE per 50 agents rather than
        // one per agent. The watchdog's stall timeout is minutes, so 50 agents
        // of granularity is far finer than it needs.
        if swept.is_multiple_of(50)
            && let Err(e) = db.touch_progress(run_id).await
        {
            tracing::warn!("heartbeat failed: {e:#}");
        }
        if swept.is_multiple_of(500) {
            tracing::info!(
                "{swept}/{remaining} agents swept this session \
                 ({unreadable} unreadable, {unwritable} unwritable this session)"
            );
        }
    }

    let finished = Utc::now();
    // Cumulative across every session this run has had, per the invariant
    // documented above the loop.
    let total_swept = already_swept.len() + swept;
    db.close_run(run_id, total_swept as i32, finished).await?;
    // Rewrite the manifest so the downloadable artefact matches the rows.
    export::write_manifest(&manifest(
        Some(total_swept),
        Some(unreadable),
        Some(unwritable),
        Some(finished.to_rfc3339()),
    ))?;
    if unreadable > 0 {
        // Say it loudly: a census missing agents is not a complete census, and
        // the gap must never be discovered later from a row count.
        tracing::warn!(
            "run {run_id}: {unreadable} of {planned} agents could not be read \
             and are ABSENT from this run — not recorded as failures"
        );
    }
    if unwritable > 0 {
        // Same principle, different failure point: these agents WERE read
        // successfully, but their database transaction never committed (a
        // permanent error, or a transient one that exhausted its retries).
        // Reported exactly like `unreadable` — loudly, at the end, never
        // discoverable only by counting rows.
        tracing::warn!(
            "run {run_id}: {unwritable} of {planned} agents were read but could not be \
             WRITTEN (database) and are ABSENT from this run — not recorded as failures"
        );
    }
    tracing::info!(
        "run {run_id} complete: {total_swept} of {planned} agents \
         ({unreadable} unreadable, {unwritable} unwritable)"
    );
    println!("{run_id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Fixtures for [`assemble_ladder`] — the P0 FIX 6 deliverable. These
    //! exercise the exact defect described in its doc comment without a
    //! database or RPC endpoint: `assemble_ladder` is pure once its inputs
    //! (already-computed `CheckResult`s and an `Option<Value>` document) are
    //! in hand.

    use super::*;
    use serde_json::json;

    const SPEC_COMMIT: &str = "68fc6765761a10fb26f0692df21c8a6f9d12b1be";
    const ACTUAL_AGENT_ID: u64 = 22;
    const ACTUAL_CHAIN_ID: u64 = 1;
    const ACTUAL_REGISTRY: &str = "0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1";

    fn t() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    fn res(rung: u8, name: &'static str, status: checks::CheckStatus) -> checks::CheckResult {
        checks::CheckResult {
            rung,
            name,
            status,
            evidence: json!({}),
            checked_at: t(),
        }
    }

    fn call(
        rung1: checks::CheckResult,
        rung2: checks::CheckResult,
        rung3: checks::CheckResult,
        document: Option<serde_json::Value>,
        rung7: Option<checks::CheckResult>,
    ) -> Vec<checks::CheckResult> {
        assemble_ladder(
            rung1,
            rung2,
            rung3,
            document,
            SPEC_COMMIT,
            ACTUAL_AGENT_ID,
            ACTUAL_CHAIN_ID,
            ACTUAL_REGISTRY.to_string(),
            rung7,
            t(),
        )
    }

    /// **The FIX 6 deliverable fixture, verbatim.** An agent that fails rung
    /// 2 (its `tokenURI()` never resolved, so there is no body and therefore
    /// no parsed document) must come back with rungs 3, 4, AND 5 all present
    /// and `skipped` — never absent, and never silently dropped the way they
    /// were before this fix. Rung 7 (`attested`) sits on the independent
    /// Reputation track and must be completely unaffected.
    #[test]
    fn a_rung_2_failure_skips_rungs_3_4_and_5_and_never_touches_attested() {
        let rung1 = res(1, "registered", checks::CheckStatus::Pass);
        let rung2 = res(2, "resolvable", checks::CheckStatus::Fail);
        // What the sweeper actually produces in this situation: rung 3 runs
        // (it is always constructed — see main()'s loop), finds no body, and
        // comes back `Error` with no document. This mirrors
        // `checks::parseable`'s real "no_body" defence-in-depth path.
        let rung3 = res(3, "parseable", checks::CheckStatus::Error);
        let rung7 = res(7, "attested", checks::CheckStatus::Pass);

        let out = call(rung1, rung2, rung3, None, Some(rung7));

        // Rung 6 stays completely absent — this fix must never invent a row
        // for the one rung that genuinely is not implemented.
        assert!(
            !out.iter().any(|r| r.rung == 6),
            "rung 6 must remain absent, not present with any status"
        );

        for rung in [3u8, 4, 5] {
            let r = out.iter().find(|r| r.rung == rung).unwrap_or_else(|| {
                panic!("rung {rung} must be present (skipped), not absent — this is the FIX 6 bug")
            });
            assert_eq!(
                r.status,
                checks::CheckStatus::Skipped,
                "rung {rung} must be skipped, not absent, not failed"
            );
            assert_eq!(
                r.evidence["skipped_because_rung"], 2,
                "rung {rung} must name rung 2 as what blocked it"
            );
        }

        let attested = out.iter().find(|r| r.rung == 7).unwrap();
        assert_eq!(
            attested.status,
            checks::CheckStatus::Pass,
            "attested sits on the independent Reputation track (depends on rung 1 \
             alone) and must be unaffected by a rung-2 failure in the Document track"
        );
        assert!(
            attested.evidence.get("skipped_because_rung").is_none(),
            "attested was never skipped, so it must carry no skip evidence"
        );
    }

    /// **The second FIX 6 deliverable fixture.** A document that parses and
    /// passes rung 4's SHOULD-only presence checks but fails rung 4's one
    /// MUST (a `registrations` entry missing `agentRegistry`) must skip rung
    /// 5, naming rung 4 as the blocker — proving skip-propagation still
    /// works within the Document track once rungs 4/5 are unconditionally
    /// constructed.
    #[test]
    fn a_rung_4_failure_skips_rung_5_naming_rung_4_as_the_blocker() {
        let rung1 = res(1, "registered", checks::CheckStatus::Pass);
        let rung2 = res(2, "resolvable", checks::CheckStatus::Pass);
        let rung3 = res(3, "parseable", checks::CheckStatus::Pass);
        // Missing `agentRegistry` on the one registrations entry — a real
        // MUST violation, so rung 4 fails for real (not overwritten: rung 3
        // passed, so rung 4's own dependency is satisfied).
        let document = json!({ "registrations": [{ "agentId": ACTUAL_AGENT_ID }] });

        let out = call(rung1, rung2, rung3, Some(document), None);

        let rung4 = out.iter().find(|r| r.rung == 4).unwrap();
        assert_eq!(
            rung4.status,
            checks::CheckStatus::Fail,
            "rung 4's own MUST violation must stand, not be overwritten"
        );

        let rung5 = out.iter().find(|r| r.rung == 5).unwrap();
        assert_eq!(rung5.status, checks::CheckStatus::Skipped);
        assert_eq!(rung5.evidence["skipped_because_rung"], 4);

        assert!(
            !out.iter().any(|r| r.rung == 6),
            "rung 6 must remain absent in every case, including this one"
        );
    }

    /// A fully-passing Document track leaves rungs 4 and 5 as real
    /// `pass`/`unclaimed` verdicts, not skipped — the unconditional
    /// construction in this fix must not turn a healthy agent into one that
    /// looks blocked.
    #[test]
    fn a_fully_passing_document_track_is_not_skipped_anywhere() {
        let rung1 = res(1, "registered", checks::CheckStatus::Pass);
        let rung2 = res(2, "resolvable", checks::CheckStatus::Pass);
        let rung3 = res(3, "parseable", checks::CheckStatus::Pass);
        let document = json!({
            "registrations": [
                { "agentId": ACTUAL_AGENT_ID, "agentRegistry": "eip155:1:0x742d35Cc6634C0532925a3b844Bc9e7595f6bEd1" }
            ],
        });

        let out = call(rung1, rung2, rung3, Some(document), None);

        let rung4 = out.iter().find(|r| r.rung == 4).unwrap();
        assert_eq!(rung4.status, checks::CheckStatus::Pass);
        let rung5 = out.iter().find(|r| r.rung == 5).unwrap();
        assert_eq!(rung5.status, checks::CheckStatus::Pass);
    }
}
