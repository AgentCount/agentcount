-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0024 — keep `check_results` vacuumed often enough to stay readable.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- Migration 0023 fixed the homepage by making the rates query index-only. That
-- fix is conditional: an index-only scan can only skip the heap where the
-- visibility map says a page is all-visible, and the visibility map is
-- maintained by vacuum. Without this file, 0023 quietly stops working a few
-- weeks after it lands, and the symptom is a homepage that 500s again with
-- nothing in any log.
--
-- The defaults cannot maintain it. At `autovacuum_vacuum_scale_factor = 0.2`,
-- autovacuum waits for dead tuples to reach 20% of the table — on 7.05 million
-- rows that is **1.41 million dead tuples**. The `refused` backfill produced
-- 229,432 and was nowhere near the threshold, so `check_results` went from
-- 2026-08-05 to 2026-08-07 unvacuumed while the homepage was down. It was not
-- neglected; it was under budget.
--
-- The insert path has the same shape: `autovacuum_vacuum_insert_scale_factor =
-- 0.2` means 1.41 million inserts before the pages a sweep just wrote are
-- marked all-visible. A BNB Chain sweep writes about 1.76 million rows, so it
-- crosses that line once, at the end, having spent the whole run invisible to
-- index-only scans.
--
-- So the thresholds are set per-table, low enough that the map tracks the
-- writes rather than trailing a whole census behind them:
--
--   * `vacuum_scale_factor 0.02` — ~141,000 dead tuples. One backfill of the
--     size that caused this outage now triggers a vacuum instead of missing the
--     threshold by a factor of six.
--   * `vacuum_insert_scale_factor 0.05` — ~352,000 inserts, so a large sweep is
--     vacuumed in several passes as it goes rather than once when it is over.
--   * `analyze_scale_factor 0.02` — the planner's row estimates decide whether
--     it even attempts the index-only scan. Stale statistics were half of why
--     it chose a sequential scan.
--
-- This costs more autovacuum work on the largest table in the database. That is
-- the intended trade: the alternative is a table whose indexes are correct and
-- whose plans do not use them.
ALTER TABLE check_results SET (
    autovacuum_vacuum_scale_factor        = 0.02,
    autovacuum_vacuum_insert_scale_factor = 0.05,
    autovacuum_analyze_scale_factor       = 0.02
);
