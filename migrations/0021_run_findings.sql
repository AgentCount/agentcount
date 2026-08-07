-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0021 — the homepage's findings, computed once per run and stored.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- `GET /api/runs/{id}/findings` answered for seven of the eight published runs
-- and, for the 2026-08 BNB Chain run (251,782 agents), did not answer at all:
-- HTTP 408, every time. The homepage sums one findings document per chain, so
-- the largest chain took the all-chains figure down with it. The headline
-- number of the census was unavailable for the largest population it measures.
--
-- ## What was actually slow, measured on production rather than guessed
--
-- The 408 is ours, not Cloud Run's: `main.rs` wraps the read routes in a
-- `TimeoutLayer` of 10 seconds. So the question is not "why does it take
-- minutes" — it is "why do six queries not finish in ten seconds", and the
-- answer is that two of them alone take fourteen.
--
-- Not a missing index. `idx_check_results_rates (run_id, rung, status)` has
-- existed since migration 0008 and four of the endpoint's six queries use it as
-- an index-ONLY scan, costing about 150 buffers each. Those four are fine and
-- were never the problem.
--
-- The other two cannot be index-only, and that is the whole story:
--
--   * the `services_absent_or_empty` numerator reads `evidence->>'services_status'`,
--     and `evidence` is a heap column no index carries; and
--   * the attested × resolvable cross-tab groups by `(chain, agent_id)` while
--     filtering on `rung`, and no single index carries all four columns —
--     `idx_check_results_rates` has (run_id, rung, status),
--     `check_results_unique` has (run_id, chain, agent_id, rung).
--
-- `EXPLAIN (ANALYZE, BUFFERS)` against the production instance (db-g1-small,
-- 1.7 GB RAM) on run e6f87cdb, 251,782 agents in a `check_results` table of
-- 7.05 million rows, with the cache warm:
--
--   rung-4 + jsonb filter   Bitmap Heap Scan   60,005 buffers   5.0 s
--   attested × resolvable   Parallel Seq Scan  267,052 buffers  9.2 s
--
-- The first is bitmap-bound for the reason the sweeper's write pattern
-- guarantees: all seven of an agent's rungs are written together, so one run's
-- rung-4 rows are spread across every heap page the run occupies — 59,275 rows
-- over 19,410 pages, three rows per 8 KB page, and rung 6's evidence (one
-- object per declared endpoint) is what makes those pages fat.
--
-- The second is worse than the bitmap case, not better: the planner declines
-- the index entirely and reads the whole table. One run is 3.6% of
-- `check_results`, and the columns needed — chain, agent_id, status, filtered
-- on rung — are not covered by any one index, so a sequential scan discarding
-- 6.5 million rows wins on cost. It then sorts 503,564 rows through an external
-- merge, spilling ~5.6 MB per worker to disk.
--
-- Fourteen seconds of database work, warm, against a ten-second request
-- budget. Cold, it is worse, and the margin on the other runs is thinner than
-- it looks: mainnet's 47,001 agents answer in 8.3 s. One more sweep and the
-- second-largest chain fails the same way.
--
-- ## Why storing the answer, and not another index
--
-- An index for each of the two — an expression index on
-- `(run_id, rung, (evidence->>'services_status'), status)` and a wider
-- `(run_id, chain, agent_id, rung, status)` — would make both index-only and
-- would genuinely fix the timeout. It was rejected for two reasons.
--
-- It is paid on every write. A sweep inserts 1.7 million `check_results` rows
-- for BNB Chain alone, and two more indexes is two more B-tree insertions per
-- row on the longest, most timeout-prone job this project runs — to make a
-- figure fast that does not change after the run finishes.
--
-- And it would still be O(rows in the run), on every page load, for every
-- chain. An index-only scan of 1.7 million entries is fast; doing it four times
-- per homepage render, forever, to recompute five numbers that were fixed the
-- moment the sweep closed, is not a thing to make fast. It is a thing to stop
-- doing.
--
-- `run_deltas` (migration 0016) settled this same argument already, in the same
-- words: a figure that is expensive and immutable is computed once by a binary
-- after the sweep and read back as rows.
--
-- ## The census's own rule, kept
--
-- A figure here must stay recomputable and pinned. So:
--
--   * every row names the run it belongs to, and cascades with it;
--   * `ls_run_findings()` below is the ONE definition of the arithmetic. The
--     backfill at the bottom of this file calls it, the `findings` binary calls
--     it, and `GET /api/runs/{id}/findings` calls it directly whenever a run
--     has no stored row. Three callers, one implementation — the same reason
--     `ls_document_fields` exists in migration 0012 rather than being written
--     out twice;
--   * nothing here is a new measurement. Every number is an aggregate over
--     `check_results`, unchanged, and re-running the function reproduces it.
--     That is what makes recomputing a findings row legitimate in a way that
--     rewriting a run's own results never is.

-- ── The arithmetic ──────────────────────────────────────────────────────────
--
-- Transcribed from `crates/api/src/routes/findings.rs` at commit 8b2391a with
-- the predicates unchanged, character for character where SQL allows. Read it
-- against that file: the five branches below are the five findings, in the
-- order the endpoint publishes them, and the reasoning for each denominator
-- lives in that module's comments and is deliberately not duplicated here.
--
-- What is NOT here: `percent`, and `denominator_label`. A percentage is
-- derived from two numbers that are both in this table, so storing it would be
-- a third number free to disagree with them; and the label is the API's
-- editorial description of a population, which belongs with the copy it is
-- written for. The endpoint keeps both.
--
-- STABLE, not IMMUTABLE: it reads tables. It is never used in an index
-- expression, so that costs nothing.
CREATE OR REPLACE FUNCTION ls_run_findings(p_run_id uuid)
RETURNS TABLE (finding_key text, numerator bigint, denominator bigint)
LANGUAGE sql STABLE AS $$
    WITH
    -- Rung 4's own evidence field, read and never recomputed from a document
    -- body. `status <> 'skipped'` rather than `status = 'pass'`: a document
    -- that failed rung 4's one conditional still either declared services or
    -- did not.
    r4 AS (
        SELECT count(*) FILTER (
                   WHERE evidence->>'services_status' IN ('absent','empty')
               ) AS services_missing,
               count(*) AS reached
        FROM check_results
        WHERE run_id = p_run_id AND rung = 4 AND status <> 'skipped'
    ),
    r4_pass AS (
        SELECT count(*) AS n FROM check_results
        WHERE run_id = p_run_id AND rung = 4 AND status = 'pass'
    ),
    r5_unclaimed AS (
        SELECT count(*) AS n FROM check_results
        WHERE run_id = p_run_id AND rung = 5 AND status = 'unclaimed'
    ),
    r7_pass AS (
        SELECT count(*) AS n FROM check_results
        WHERE run_id = p_run_id AND rung = 7 AND status = 'pass'
    ),
    agents AS (
        SELECT count(*) AS n FROM agent_snapshots WHERE run_id = p_run_id
    ),
    -- One grouped pass, not a self-join, for the reason recorded in
    -- findings.rs: the join compared every rung-7 row against every rung-2 row
    -- in the run. `check_results_unique (run_id, chain, agent_id, rung)`
    -- guarantees at most one row per rung per agent, so `max(status)` has
    -- nothing to choose between — it is "that row's status".
    per_agent AS (
        SELECT max(status) FILTER (WHERE rung = 7) AS r7,
               max(status) FILTER (WHERE rung = 2) AS r2
        FROM check_results
        WHERE run_id = p_run_id AND rung IN (2, 7)
        GROUP BY chain, agent_id
    ),
    cross_tab AS (
        SELECT count(*) FILTER (WHERE r7 = 'pass') AS att_total,
               count(*) FILTER (WHERE r7 = 'pass' AND r2 = 'pass') AS att_resolvable,
               count(*) FILTER (WHERE r7 <> 'pass') AS unatt_total,
               count(*) FILTER (WHERE r7 <> 'pass' AND r2 = 'pass') AS unatt_resolvable
        FROM per_agent
        WHERE r7 IS NOT NULL AND r2 IS NOT NULL
    )
    SELECT 'services_absent_or_empty', r4.services_missing, r4.reached FROM r4
    UNION ALL
    SELECT 'registration_unclaimed', r5_unclaimed.n, r4_pass.n
      FROM r5_unclaimed, r4_pass
    UNION ALL
    SELECT 'attested', r7_pass.n, agents.n FROM r7_pass, agents
    UNION ALL
    SELECT 'attested_resolvable', cross_tab.att_resolvable, cross_tab.att_total
      FROM cross_tab
    UNION ALL
    SELECT 'unattested_resolvable', cross_tab.unatt_resolvable, cross_tab.unatt_total
      FROM cross_tab
$$;

-- ── The rows ────────────────────────────────────────────────────────────────
--
-- One row per (run, finding). Rows rather than a column per finding, because
-- the set of findings grows: three more keys are already approved, and adding
-- one should be a line in the function and a line in the endpoint, not a
-- migration.
CREATE TABLE IF NOT EXISTS run_findings (
    -- The run this figure is a claim about. There is no such thing as a
    -- findings row that is not pinned to one, and deleting a run takes its
    -- findings with it — a figure outliving the measurement it summarises is
    -- how an unfalsifiable number gets published.
    run_id      uuid        NOT NULL REFERENCES runs(run_id) ON DELETE CASCADE,
    -- The endpoint's stable key. Not a foreign key to anything: the canonical
    -- list of findings, their order and their denominator labels live in
    -- `crates/api/src/routes/findings.rs`, which is the only place that decides
    -- what the census publishes. This table supplies two numbers per key and
    -- has no opinion about which keys exist.
    finding_key text        NOT NULL,
    numerator   bigint      NOT NULL,
    denominator bigint      NOT NULL,
    -- When this row was derived — NOT when the run was swept. The two differ
    -- whenever a finding is recomputed, and a reader comparing a published
    -- figure against a fresh count needs to know which is which.
    computed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, finding_key)
);

-- ── Backfill ────────────────────────────────────────────────────────────────
--
-- Every run that has results, through the same function the sweep will use.
--
-- THIS IS SLOW, ON PURPOSE, ONCE. It performs exactly the work the endpoint
-- used to do per request — about 2.5 GB of reads for a BNB Chain run — for
-- every run in the database, and there are runs here that were never published.
-- On the production instance the two heavy queries measured 14 s for the
-- largest run with the cache warm; budget minutes per large run and something
-- on the order of half an hour in total, in one transaction, cold.
-- Migrations here are applied by hand rather than at container
-- start (nothing in `Dockerfile` or `scripts/` runs `sqlx migrate run`), so
-- that is a supervised wait and not a failed deploy. Run it when a sweep is
-- not.
--
-- An operator who would rather not hold one transaction open that long can run
-- `findings --all` against the previous schema-plus-function first and then
-- apply this file: the `ON CONFLICT DO NOTHING` below makes the backfill a
-- no-op for anything already computed. THE API MUST BE DEPLOYED AFTER THIS
-- MIGRATION, not before — the new handler reads `run_findings` and
-- `ls_run_findings()`, and neither exists until this runs.
--
-- Runs with no `check_results` at all are skipped rather than written as
-- zeroes: a run that was opened and never swept has no findings, and a row of
-- zeroes would read as "we asked and found none".
INSERT INTO run_findings (run_id, finding_key, numerator, denominator)
SELECT r.run_id, f.finding_key, f.numerator, f.denominator
FROM runs r
CROSS JOIN LATERAL ls_run_findings(r.run_id) f
WHERE EXISTS (SELECT 1 FROM check_results c WHERE c.run_id = r.run_id)
ON CONFLICT (run_id, finding_key) DO NOTHING;
