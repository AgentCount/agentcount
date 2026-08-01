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
done

if [ -n "$failed" ]; then
    echo
    echo "FAILED:$failed"
    exit 1
fi
echo
echo "all chains complete"
