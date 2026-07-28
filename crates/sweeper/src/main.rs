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
//! `conformant`, `bound`). Day 3 adds rung 7 (`independent`), constructed
//! ONLY for the agents that reach it — those that passed rungs 1-5 — so a
//! Reputation Registry read (`chain::Reputation::feedback`) happens for the
//! ~2% of the population it can possibly matter for, never all of it. Rung 6
//! is still ABSENT from the output rather than reported as `skipped` — "we
//! did not ask" and "we could not ask" are different claims and the schema
//! keeps them different.
//!
//! Two independent concurrency budgets drive the pipeline, on purpose (see
//! [`rpc_concurrency`] and [`fetch_concurrency`]): the RPC endpoint throttles
//! hard (a public free-tier provider), while HTTP fetches are limited
//! per-host by `probe` itself. Collapsing them into one shared number would
//! mean tuning one starves the other.

mod export;
mod store;

use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::Utc;
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

/// The published contact string from `METHODOLOGY.md` (search
/// `ledgerscope-probe`) — the single source for the User-Agent's contact
/// portion. Declared here, not in `crates/probe`, and passed into
/// [`probe::Prober::new`] as a parameter, so the crate that actually sends
/// the header never hardcodes it and cannot drift from what METHODOLOGY.md
/// promises.
const PROBE_CONTACT_URL: &str =
    "https://ledgerscope.io/methodology; contact: probes@ledgerscope.io";

/// HTTPS gateway `ipfs://` URIs are rewritten onto before fetching.
/// Overridable via `IPFS_GATEWAY` for anyone who wants a different (or
/// self-hosted) gateway; the evidence records which one served each agent
/// (`via_gateway`) so a reader can tell an agent's failure from the
/// gateway's.
fn ipfs_gateway() -> String {
    std::env::var("IPFS_GATEWAY").unwrap_or_else(|_| "https://ipfs.io/ipfs/".to_string())
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
fn checks_scheme(outcome: &probe::FetchOutcome) -> String {
    if outcome.scheme.is_empty() {
        "empty".to_string()
    } else if outcome.request_url.is_some() {
        // A real HTTP(s) request was attempted (http, https, or ipfs via the
        // gateway) — keep whichever of those labels probe already assigned.
        outcome.scheme.clone()
    } else if outcome.scheme == "data" && outcome.body.is_some() {
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

#[tokio::main]
async fn main() -> Result<()> {
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
    let gateway = ipfs_gateway();
    let prober = probe::Prober::new(PROBE_CONTACT_URL, &gateway)?;

    // `deploy_block` is no longer used for enumeration (agent ids are found
    // by binary search on `ownerOf` existence, not by scanning logs from
    // deploy to head — see crates/chain/src/registry.rs), but the column
    // still describes the chain and stays wired for chain_config's other
    // callers.
    let _ = deploy_block;

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
        .buffer_unordered(fetch_concurrency());

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
        let (s, outcome) = match result {
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
                // The registration tx lives in raw_events from the indexer;
                // wiring it in is Day 2 work. Null, never invented.
                tx_hash: None,
            },
            now,
        );

        let inline_bytes = if scheme == "data" {
            outcome.body.as_ref().map(Vec::len)
        } else {
            None
        };
        // The URI goes into rung 2's evidence, which is a `jsonb` column, and
        // Postgres rejects ` ` in jsonb outright — so the same escape the
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

        // Rungs 4 and 5 need a parsed document, and only rung 3 produces one.
        // When `document` is `None` they are simply not constructed — NOT
        // constructed-and-failed, and NOT marked `Skipped` here either: that
        // is `run_ladder`'s job alone (see the module doc). A document that
        // never parsed cannot be judged missing a field or unbound; it can
        // only be a question this ladder never got to ask.
        let rung4 = document.as_ref().map(|doc| {
            checks::conformant(
                &checks::ConformantInput {
                    document: doc.clone(),
                },
                spec_commit,
                now,
            )
        });
        let rung5 = document.as_ref().map(|doc| {
            checks::bound(
                &checks::BoundInput {
                    document: doc.clone(),
                    // The SAME chain_id/registry rung 1 was judged against —
                    // one source, so the two rungs can never disagree about
                    // what "on-chain reality" was for this agent.
                    actual_agent_id: s.agent_id,
                    actual_chain_id: chain_id as u64,
                    actual_registry: registry_addr.clone(),
                },
                now,
            )
        });

        // Rung 7 sits ABOVE rung 5: in the reference census only 1,425 of
        // 60,037 agents pass rungs 1-5, so gating the Reputation Registry
        // read on that (rather than always reading it and letting
        // `run_ladder` discard the result) is what keeps the sweep's RPC
        // cost near ~1,425 extra call pairs instead of ~120,000. Checked via
        // `.as_ref()` so `rung4`/`rung5` are not yet moved — they still need
        // to go into `rungs` below.
        let reaches_rung7 = rung1.status == checks::CheckStatus::Pass
            && rung2.status == checks::CheckStatus::Pass
            && rung3.status == checks::CheckStatus::Pass
            && rung4
                .as_ref()
                .is_some_and(|r| r.status == checks::CheckStatus::Pass)
            && rung5
                .as_ref()
                .is_some_and(|r| r.status == checks::CheckStatus::Pass);

        // `None` here means one of two different things, and the branches
        // below keep them distinct:
        //   - rungs 1-5 didn't all pass → rung 7 is simply not asked, and
        //     `run_ladder` will mark it `Skipped` on its own (never
        //     synthesised here — see the module doc on `run_ladder`).
        //   - rungs 1-5 DID all pass but the feedback read itself failed →
        //     that's OUR problem, not the agent's, so — same as an
        //     unreadable snapshot above — this agent is left out of the run
        //     entirely (`continue`) rather than recording anything false
        //     about it.
        let rung7 = if !reaches_rung7 {
            None
        } else if let Some(rep) = &reputation {
            match rep.feedback(s.agent_id, pinned).await {
                Ok(fr) => Some(checks::independent(
                    &checks::IndependentInput {
                        owner: s.owner.clone(),
                        clients: fr.clients,
                        feedback_count: fr.feedback_count,
                        registry_available: true,
                    },
                    now,
                )),
                Err(e) => {
                    tracing::warn!(
                        "agent {}: reputation feedback read failed: {e:#} — leaving this \
                         agent out of the run rather than recording anything false about it",
                        s.agent_id
                    );
                    unreadable += 1;
                    continue;
                }
            }
        } else {
            // `chains.reputation_registry` is NULL for this chain: we cannot
            // check, which is our limitation, not the agent's — `Error`,
            // never `Fail`. No RPC call is made; `checks::independent` alone
            // decides the status from `registry_available: false`.
            Some(checks::independent(
                &checks::IndependentInput {
                    owner: s.owner.clone(),
                    clients: Vec::new(),
                    feedback_count: 0,
                    registry_available: false,
                },
                now,
            ))
        };

        let mut rungs = vec![rung1, rung2, rung3];
        if let Some(r4) = rung4 {
            rungs.push(r4);
        }
        if let Some(r5) = rung5 {
            rungs.push(r5);
        }
        if let Some(r7) = rung7 {
            rungs.push(r7);
        }
        let results = checks::run_ladder(rungs);

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
        if swept % 500 == 0 {
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
