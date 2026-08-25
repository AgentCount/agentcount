#!/usr/bin/env python3
"""Fail the build on a per-agent query that names `run_id` but not `chain`.

`agent_snapshots` and `http_archive` are keyed (run_id, chain, agent_id, ...).
`chain` sits between `run_id` and `agent_id` even though a run has exactly one
chain, so a predicate that gives Postgres `run_id` and `agent_id` but not
`chain` can seek on the leading column only and then walks every row the run
wrote.

`check_results` WAS the third such table and is deliberately no longer checked.
Migration 0025 replaced its unique key `(run_id, chain, agent_id, rung)` with
`(run_id, agent_id, rung)`, and no remaining index on it leads with `chain`, so
a query naming `run_id` and `agent_id` seeks correctly without the column. That
was the migration's whole purpose — "this removes the reason anyone has to" —
and continuing to demand the predicate here would fail builds for a query that
is already optimal, on a rationale that stopped being true on 2026-08-11. If a
future index on `check_results` leads with `chain` again, add it back.

That is not a small penalty and it is not theoretical. Measured on production
against the 2026-08 BNB Chain run (251,782 agents, 1.76 million rows):

    /api/agents page of 50     278,731 buffers   8,915 ms  ->  223 buffers   8.8 ms
    rung-6 delete, per agent    19,001 buffers   1,549 ms  ->   10 buffers   1.7 ms
    refused backfill chunk      timed out >120 s           ->                74 ms

Six queries were written this way before anyone noticed, because the column is
redundant — a run IS one chain — so leaving it out reads as tidy rather than
wrong. Two of them shipped: the agent directory returned 408 for BNB Chain, and
the rung-6 pass would have needed roughly 36 hours for a run that takes two
minutes with the column present.

The check is deliberately crude. It flags a SQL string literal that mentions one
of these tables, constrains `agent_id`, constrains `run_id`, and never mentions
`chain`. False positives are possible; the fix for one is to add the predicate
anyway, because it is free and correct.
"""

import os
import re
import sys

TABLES = ("agent_snapshots", "http_archive")
ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "crates")


def offending(path: str):
    src = open(path, encoding="utf-8", errors="replace").read()
    for m in re.finditer(r'"((?:[^"\\]|\\.)*)"', src, re.S):
        # Rust splits long SQL across lines with a trailing backslash; join it
        # back before matching so a predicate broken over two lines still reads
        # as one statement.
        q = re.sub(r"\\\s*", " ", m.group(1))
        q = re.sub(r"\s+", " ", q).strip()
        if not any(t in q for t in TABLES):
            continue
        if not re.search(r"\b(SELECT|DELETE|UPDATE)\b", q, re.I):
            continue
        # Per-agent: the query pins specific agents rather than scanning a run.
        if not re.search(r"agent_id\s*(=|\bIN\b|=\s*ANY)", q, re.I):
            continue
        if not re.search(r"run_id\s*=", q, re.I):
            continue
        if re.search(r"\bchain\s*=", q, re.I):
            continue
        yield src[: m.start()].count("\n") + 1, q


def main() -> int:
    hits = []
    for root, _, files in os.walk(ROOT):
        for f in files:
            if f.endswith(".rs"):
                p = os.path.join(root, f)
                for line, q in offending(p):
                    hits.append((os.path.relpath(p, os.path.join(ROOT, "..")), line, q))

    if not hits:
        print("no per-agent query omits `chain`")
        return 0

    for path, line, q in hits:
        print(f"::error file={path},line={line}::query names run_id and agent_id but not chain")
        print(f"  {path}:{line}")
        print(f"    {q[:160]}")
    print()
    print(f"{len(hits)} query/queries would scan a whole run instead of seeking.")
    print("Add `AND chain = $n`. The caller already knows it — a run is one chain.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
