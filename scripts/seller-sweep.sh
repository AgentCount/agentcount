#!/usr/bin/env bash
# One week's Seller Census: every catalog, every rung this sweep asks.
#
# The sibling of `weekly-sweep.sh` and equally runnable from a workstation.
# Four binaries, in this order and for these reasons:
#
#   seller-crawl   fetches every catalog, hashes what each served, and
#                  assembles the population. Rung 1.
#   seller-probe   one request per seller for rungs 2, 3 and 7. Must run
#                  after the population exists, because the unit of politeness
#                  is a HOST and per-host budgets need the whole population in
#                  hand before the first request goes out.
#   seller-settle  rung 6, from the chain. Independent of the probe, but run
#                  after it so a failed probe does not cost an RPC scan.
#   seller-delta   compares this sweep against the previous FINISHED one, so
#                  it must run last, after every rung this sweep attempted.
#
# ── What this sweep does NOT do ──────────────────────────────────────────────
#
# **Rung 4 does not run.** The mystery shopper spends real money, and the
# wallet in METHODOLOGY §10.4 is deliberately unfunded until the instrument's
# packaging is settled. That is a legitimate sweep and it is legible as one:
# rung 4 gets NO ROWS — never a zero, never a `fail` — and `rungs_attempted`
# records what was asked, so the delta marks the sweep that later adds it as a
# METHOD change rather than letting a delivery rate appear from nowhere.
#
# To run it, once the wallet is funded, add `seller-shop` to this script and
# nothing else changes.
#
# ── Failure is not retried here ──────────────────────────────────────────────
#
# A stage that fails leaves its rows and stops the sweep, exactly as the
# registration census does. A crawl that could not read every catalog marks
# its run `failed`, and `seller-delta` refuses to compare against a failed
# run — that rule exists because the first production delta reported 2,387
# sellers as having "appeared" when they had merely never been counted.
set -euo pipefail

: "${DATABASE_URL:?set DATABASE_URL}"

echo "═══════════════════════════════════════════ sellers: crawl"
seller-crawl

echo "═══════════════════════════════════════════ sellers: probe (rungs 2, 3, 7)"
seller-probe

echo "═══════════════════════════════════════════ sellers: settle (rung 6)"
# Needs an RPC endpoint. Skipped rather than failed when none is configured:
# a sweep without rung 6 is a sweep that did not ask, which `rungs_attempted`
# already knows how to say, and it should not cost the three rungs that did.
if [ -n "${RPC_URL_BASE:-}" ]; then
    seller-settle
else
    echo "!!! RPC_URL_BASE unset — rung 6 not attempted this sweep"
fi

echo "═══════════════════════════════════════════ sellers: delta"
seller-delta

echo
echo "Done. Rung 4 was not attempted; see this script's header."
