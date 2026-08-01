#!/usr/bin/env bash
# One week's census: every chain, the full ladder, and the delta.
#
# The entrypoint of `Dockerfile.sweep`, and equally runnable from a
# workstation. Three binaries per chain, in this order and for these reasons:
#
#   sweeper   pins a block and answers rungs 1-5 and 7
#   liveness  rung 6 — probes the endpoints those documents declared, so it
#             must run after the documents are archived
#   delta     compares the finished run against the previous one, so it must
#             run after the run is closed
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
#
# RPC URLs carry API keys. Nothing here echoes one, and nothing should be
# added that does — `set -x` in particular would put every key in the job log.
set -uo pipefail

CHAINS="${SWEEP_CHAINS:-base celo mainnet bsc}"
# Cheapest chain first, most expensive last. If the job is going to hit a
# timeout it should do so having already published three censuses rather than
# having spent the whole window on the 244,208-agent one.

: "${DATABASE_URL:?DATABASE_URL must be set}"

failed=""
for chain in $CHAINS; do
    rpc_var="RPC_URL_$(echo "$chain" | tr '[:lower:]' '[:upper:]')"
    if [ -z "${!rpc_var:-}" ]; then
        echo "!!! $chain: $rpc_var is not set — skipping this chain"
        failed="$failed $chain(no-rpc)"
        continue
    fi

    echo "═══════════════════════════════════════════ $chain: sweep"
    if ! sweeper "$chain"; then
        # The sweeper exits 75 specifically when its own stall watchdog fires.
        # Distinguished in the log because the two failures want different
        # responses: a stall is usually the network or the machine, an
        # ordinary failure is usually the chain or the database.
        code=$?
        [ "$code" = 75 ] && echo "!!! $chain: STALLED (watchdog)" || echo "!!! $chain: sweep exited $code"
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

# ── The git summary, committed BEFORE the heartbeat ──────────────────────────
#
# A hash that only exists on a server we control is not evidence. Committing it
# is what makes the archive attestable, so it happens before anything reports
# the week healthy.
if git -C . diff --quiet -- published-runs.json; then
    echo "published-runs.json unchanged — nothing new to commit"
else
    echo "═══════════════════════════════════════════ committing the run summary"
    git add published-runs.json
    git -c user.name="agentcount-sweep" -c user.email="probes@agentcount.ai" \
        commit -q -m "data: publish $(date -u +%Y-%m-%d) runs" \
        && git push -q origin HEAD \
        || { echo "!!! could not commit/push published-runs.json"; failed="$failed git-summary"; }
fi

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
