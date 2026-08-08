#!/usr/bin/env bash
# One week's census: every chain, the full ladder, and the delta.
#
# The entrypoint of `Dockerfile.sweep`, and equally runnable from a
# workstation. Five binaries per chain, in this order and for these reasons:
#
#   sweeper   pins a block and answers rungs 1-5 and 7
#   liveness  rung 6 — probes the endpoints those documents declared, so it
#             must run after the documents are archived
#   delta     compares the finished run against the previous one, so it must
#             run after the run is closed
#   findings  counts the homepage's five figures once, so that rendering them
#             does not mean counting 1.7 million rows per page load
#   payments  reads token transfer logs at the run's pinned block — NOT a rung
#
# `payments` is OFF by default. It is the longest step, it costs one
# `getAgentWallet` call per agent (244,208 of them on BNB Chain), and no
# published figure depends on it yet — see METHODOLOGY.md §8. Set
# `SWEEP_PAYMENTS=1` to include it. When it is off, a chain simply has no
# payment rows for the week, and `payment_scans` having no row is how a reader
# tells "not scanned" from "scanned and found nothing".
#
# ## Failure is per chain, not per run
#
# A chain that fails does NOT stop the others. Four independent censuses share
# a schedule, not a fate — losing BNB Chain to an RPC outage should not also
# cost Base, and the alternative (abort on first error) means the most fragile
# chain decides whether anything gets published each week.
#
# The exit code is non-zero if ANY chain failed, so the scheduler still sees a
# failure and the alert still fires. What is lost is only the work that could
# not be done.
#
# ## Environment
#
#   DATABASE_URL            required
#   RPC_URL_BASE, _BSC, _MAINNET, _CELO   required per chain swept
#   SWEEP_CHAINS            optional, space-separated; defaults to all four
#   SWEEP_PAYMENTS          optional; 1 runs the payments pass, default off
#   SWEEP_REJUDGE           optional; run ids, or `all`. Re-judges rung 6 on
#                           EXISTING runs and does nothing else — see below
#
# RPC URLs carry API keys. Nothing here echoes one, and nothing should be
# added that does — `set -x` in particular would put every key in the job log.
set -uo pipefail

CHAINS="${SWEEP_CHAINS:-op polygon arbitrum gnosis celo xlayer megaeth billions mainnet base bsc}"
# Cheapest chain first, most expensive last. If the job is going to hit a
# timeout it should do so having already published three censuses rather than
# having spent the whole window on the 244,208-agent one.

: "${DATABASE_URL:?DATABASE_URL must be set}"

# ── Re-judge mode: rung 6 over runs that already exist ───────────────────────
#
#   SWEEP_REJUDGE=all                       every finished run carrying rung 6
#   SWEEP_REJUDGE="<run-id> <run-id> …"     exactly these
#
# When rung 6's rules change, every published run still carries the old words
# until something re-reads them. On 2026-08-06 rung 6 gained `refused` and five
# runs needed re-judging; that was done by hand, as four `gcloud run jobs
# update && execute` pairs typed one after another, which is how a step gets
# skipped or given the wrong run id at midnight. This is that sequence, once,
# in the same place as everything else the census does.
#
# **It sweeps nothing and publishes nothing.** No block is pinned, no archive is
# written, no heartbeat is sent. A re-judge is not a census: it re-reads
# evidence already in the database and re-answers one rung from it.
#
# ## What it does and does not send
#
# `liveness` probes only URLs it has not already probed for that run, so a
# re-judge of a completed pass sends NOTHING — all five runs on 2026-08-07
# logged `0 URLs to probe`. That is a property of the data, not a promise of
# this script: if rung 6's host budget or endpoint selection has changed since
# the run, the newly-selected URLs have no archived probe and WILL be fetched.
# Check the `URLs to probe` line in the log before assuming a quiet pass.
#
# `findings` follows each run because the stored figures summarise rung 6 among
# other things, and leaving them describing the old verdicts is how the homepage
# comes to disagree with the run behind it.
if [ -n "${SWEEP_REJUDGE:-}" ]; then
    if [ "$SWEEP_REJUDGE" = "all" ]; then
        # Every finished run that HAS rung-6 rows. A run without them was swept
        # before rung 6 existed, and inventing a verdict for it now would be a
        # measurement, not a re-judgement.
        REJUDGE_RUNS=$(psql "$DATABASE_URL" -tAc \
            "SELECT DISTINCT r.run_id FROM runs r JOIN check_results c USING (run_id) \
             WHERE c.rung = 6 AND r.status = 'finished' ORDER BY 1" 2>/dev/null)
    else
        REJUDGE_RUNS="$SWEEP_REJUDGE"
    fi

    if [ -z "$REJUDGE_RUNS" ]; then
        echo "SWEEP_REJUDGE set but no run matched — nothing to do"
        exit 1
    fi

    rejudge_failed=""
    for run_id in $REJUDGE_RUNS; do
        # Shape-checked before it reaches SQL. These ids come from an operator
        # typing an environment variable, so this is a typo guard rather than a
        # security boundary — but the id is interpolated into a query below, and
        # a value that is not a uuid should be rejected by name here instead of
        # becoming a psql syntax error twenty lines later.
        case "$run_id" in
            [0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]-[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]-[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]-[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]-[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]) ;;
            *)
                echo "!!! $run_id: not a run id"
                rejudge_failed="$rejudge_failed $run_id(malformed)"
                continue
                ;;
        esac

        # The chain comes from the run, never from the caller: `liveness` takes
        # both, and a mismatched pair would re-judge the wrong population.
        chain=$(psql "$DATABASE_URL" -tAc \
            "SELECT chain FROM runs WHERE run_id = '$run_id'" 2>/dev/null | tr -d '[:space:]')
        if [ -z "$chain" ]; then
            echo "!!! $run_id: no such run"
            rejudge_failed="$rejudge_failed $run_id(unknown)"
            continue
        fi

        echo "═══════════════════════════════════════════ $chain $run_id: rung 6 re-judge"
        liveness "$chain" "$run_id" \
            || { echo "!!! $run_id: liveness exited $?"; rejudge_failed="$rejudge_failed $run_id(rung6)"; continue; }

        echo "═══════════════════════════════════════════ $chain $run_id: findings"
        findings "$run_id" \
            || { echo "!!! $run_id: findings exited $?"; rejudge_failed="$rejudge_failed $run_id(findings)"; }
    done

    if [ -n "$rejudge_failed" ]; then
        echo
        echo "REJUDGE FAILED:$rejudge_failed"
        exit 1
    fi
    echo
    echo "rung 6 re-judged for every run named"
    exit 0
fi

failed=""
for chain in $CHAINS; do
    rpc_var="RPC_URL_$(echo "$chain" | tr '[:lower:]' '[:upper:]')"
    if [ -z "${!rpc_var:-}" ]; then
        echo "!!! $chain: $rpc_var is not set — skipping this chain"
        failed="$failed $chain(no-rpc)"
        continue
    fi

    echo "═══════════════════════════════════════════ $chain: sweep"
    # `$?` is read on the line straight after the command and NOWHERE else.
    # This used to be `if ! sweeper "$chain"; then code=$?`, which always
    # reported 0: inside the branch, `$?` is the status of the `if` condition
    # — and `!` had already inverted the failure into a success. So on
    # 2026-08-04, when three chains were killed by the stall watchdog, the log
    # said `sweep exited 0` for every one of them and never once said STALLED.
    # A misleading log during an incident is worse than no log.
    sweeper "$chain"
    code=$?
    if [ "$code" -ne 0 ]; then
        # The sweeper exits 75 specifically when its own stall watchdog fires.
        # Distinguished in the log because the two failures want different
        # responses: a stall is usually the network or the machine, an
        # ordinary failure is usually the chain or the database.
        if [ "$code" -eq 75 ]; then
            echo "!!! $chain: STALLED (watchdog, exit 75)"
        else
            echo "!!! $chain: sweep exited $code"
        fi
        failed="$failed $chain(sweep)"
        # No rung 6 and no delta for a chain whose sweep did not finish —
        # `liveness` requires a finished run and `delta` would compare against
        # the wrong pair.
        continue
    fi

    echo "═══════════════════════════════════════════ $chain: rung 6"
    # Rung 6 failing does not invalidate the sweep. The run keeps rungs 1-5
    # and 7, and rung 6 is simply absent for that chain this week — which is
    # what an absent rung has always meant here.
    liveness "$chain" || { echo "!!! $chain: liveness exited $?"; failed="$failed $chain(rung6)"; }

    echo "═══════════════════════════════════════════ $chain: delta"
    delta "$chain" || { echo "!!! $chain: delta exited $?"; failed="$failed $chain(delta)"; }

    echo "═══════════════════════════════════════════ $chain: findings"
    # After rung 6, so the stored figures describe the run as it will be
    # published. Tolerated like rung 6 and the delta: a chain that fails here
    # keeps its census, and `/api/runs/{id}/findings` falls back to counting
    # the run live — which is the behaviour that timed out on BNB Chain, so a
    # failure here IS the launch blocker returning for that chain and the
    # non-zero exit below is how the scheduler says so.
    findings "$chain" || { echo "!!! $chain: findings exited $?"; failed="$failed $chain(findings)"; }

    # Payments, opt-in. Tolerated exactly like rung 6: a failure here does not
    # invalidate the sweep, because no rung and no published rate reads these
    # rows. The chain keeps its census and simply has no payment rows this
    # week — which the schema states rather than implies.
    if [ "${SWEEP_PAYMENTS:-0}" = "1" ]; then
        echo "═══════════════════════════════════════════ $chain: payments"
        payments "$chain" || { echo "!!! $chain: payments exited $?"; failed="$failed $chain(payments)"; }
    fi

    echo "═══════════════════════════════════════════ $chain: publish"
    # The run id this pass just finished, asked of the database rather than
    # remembered — the sweeper may have resumed an existing run rather than
    # opening a new one, and a remembered id would publish the wrong week.
    run_id=$(psql "$DATABASE_URL" -tAc \
        "SELECT run_id FROM runs WHERE chain = '$chain' AND status = 'finished' \
         ORDER BY finished_at DESC LIMIT 1" 2>/dev/null | tr -d '[:space:]')
    if [ -z "$run_id" ]; then
        echo "!!! $chain: no finished run to publish"
        failed="$failed $chain(publish)"
        continue
    fi
    # Export → archive → checksum → upload → record the hash for git. All of it
    # before the heartbeat below, which is the point: the ping means "this
    # week's data is published and verifiable", not "the process exited".
    export-run "$run_id" \
        && ./scripts/publish-run.sh "$run_id" \
        || { echo "!!! $chain: publish failed"; failed="$failed $chain(publish)"; }
done

# ── The index is published; the COMMIT is a human step ───────────────────────
#
# `publish-run.sh` uploads `runs/index.json` to the bucket, which is what
# `heartbeat` below verifies and what anyone can fetch. It does NOT commit the
# hashes to git, and this job deliberately cannot:
#
#   * the image carries binaries and scripts, not a checkout, so there is
#     nothing here to commit into; and
#   * giving an unattended weekly job write access to the source repository, to
#     record a hash, is a worse trade than typing one command afterwards.
#
# Git is still where a hash becomes EVIDENCE — a value in a commit predating
# any dispute — so the commit happens when the week's report is written:
#
#     gsutil cp gs://agentcount-data/runs/index.json published-runs.json
#     git add published-runs.json && git commit -m "data: publish <date> runs"
#
# A divergence between the bucket and git is visible to anyone who compares
# them, which is the property that matters.

if [ -n "$failed" ]; then
    echo
    echo "FAILED:$failed"
    # No heartbeat. The monitor alerts on silence, so the correct thing to do
    # when anything went wrong is to say nothing to it.
    exit 1
fi

# ── The dead man's switch, LAST ──────────────────────────────────────────────
#
# Everything above can fail loudly. The one failure that cannot is the schedule
# never firing at all — no log line, no exit code, because nothing ran. The only
# signal for that is an absence, and an absence can only be noticed from
# outside. `heartbeat` re-reads the published index from disk rather than
# trusting the steps above: a step that reported success but wrote nothing is
# exactly what this is for.
echo
echo "═══════════════════════════════════════════ heartbeat"
heartbeat || { echo "!!! heartbeat declined to ping — the census is NOT healthy"; exit 1; }

echo
echo "all chains complete and published"
