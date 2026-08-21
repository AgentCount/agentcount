-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0027 — what changed between two seller sweeps, and which
-- questions each sweep actually asked.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- METHODOLOGY §10.6. The delta ships with the instrument rather than four
-- migrations later, because the registration census learned the hard way that
-- a series computed after the fact is a series computed differently each time
-- somebody asks.
--
-- ## Why `rungs_attempted` is here and not in 0026
--
-- 0026 is applied and public; migrations are an append-only record and are not
-- rewritten after the fact, however convenient. This is the same reason
-- `backfill-refused` restamps rather than edits.
--
-- The column exists because **a sweep may honestly skip a rung.** The first
-- production sweep runs rungs 1, 2, 3, 6 and 7 and does NOT run rung 4: the
-- mystery shopper spends real money, and the wallet is deliberately unfunded
-- until the instrument's packaging is settled. That is a legitimate sweep, and
-- it must be legible as one:
--
--   * a rung nobody asked has NO ROWS — never a row saying `fail`, and never a
--     zero. Absence is not a status, here as everywhere;
--   * "no rows" and "the pass crashed" must be distinguishable, which is what
--     this column does — it records what the sweep SET OUT to ask;
--   * a later sweep that adds rung 4 differs from this one by METHOD, and the
--     delta must say so rather than publishing a delivery rate that appears
--     from nowhere.
--
-- A run with a NULL here predates the column: unknown, not empty.
ALTER TABLE seller_runs ADD COLUMN rungs_attempted SMALLINT[];

COMMENT ON COLUMN seller_runs.rungs_attempted IS
    'Which rungs this sweep set out to ask. A rung absent here was never '
    'attempted, which is different from attempted-and-empty. NULL predates '
    'the column.';

-- ─────────────────────────────────────────────────────────────────────────────
-- seller_run_deltas — one row per (newer sweep), naming the pair.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- Shaped after `run_deltas` (0016) and carrying the same rules, plus the two
-- confounds a seller population has that an on-chain one does not.
CREATE TABLE seller_run_deltas (
    -- The newer sweep. One delta per sweep, replacing any earlier computation
    -- of it — a delta is derived, so recomputing it after a rule change is
    -- legitimate in a way that rewriting a sweep's own results never is.
    run_id              UUID        PRIMARY KEY REFERENCES seller_runs (run_id) ON DELETE CASCADE,
    -- What it is compared against. A sweep with no predecessor gets NO ROW at
    -- all: "first observation" and "nothing changed" are different claims.
    previous_run_id     UUID        NOT NULL REFERENCES seller_runs (run_id) ON DELETE CASCADE,
    network             TEXT        NOT NULL,

    sellers_before      INTEGER     NOT NULL,
    sellers_after       INTEGER     NOT NULL,
    appeared            INTEGER     NOT NULL,
    -- A seller "disappears" when no catalog lists it any more, which is a fact
    -- about the catalogs as much as about the seller — hence the confound
    -- columns below.
    disappeared         INTEGER     NOT NULL,

    -- The two headline series. Rung 2 moved to/from `pass`, EXCLUDING every
    -- transition that touches `refused`, `error` or `unprobed` — the rule
    -- `sellers::delta` holds, inherited from §9's two incidents plus this
    -- instrument's own word for "we did not ask".
    came_back           INTEGER     NOT NULL,
    went_dark           INTEGER     NOT NULL,

    -- The excluded volumes, by kind, so no exclusion is silent and the three
    -- can be published beside the series that benefit from them. Additive: a
    -- transition is counted under exactly one.
    excluded_refused    INTEGER     NOT NULL,
    excluded_error      INTEGER     NOT NULL,
    excluded_unprobed   INTEGER     NOT NULL,

    -- Every transition, including the excluded ones:
    -- `[{"rung","from","to","sellers"}, …]`, sorted.
    flips               JSONB       NOT NULL,

    -- ── The confound, and the rule for publishing ───────────────────────
    --
    -- Three ways the method can move under a seller population, where the
    -- registration census has one. Any surface rendering this delta must say
    -- when any of them is true.
    checker_before      TEXT        NOT NULL,
    checker_after       TEXT        NOT NULL,
    -- The catalog lists of the two sweeps. A seller that vanished because its
    -- only catalog was dropped is a method change, not churn.
    catalogs_before     TEXT[]      NOT NULL,
    catalogs_after      TEXT[]      NOT NULL,
    -- The questions each sweep asked. A delivery rate that "appears" because
    -- the later sweep ran the shopper is method, not world.
    rungs_before        SMALLINT[],
    rungs_after         SMALLINT[],
    -- True iff any of the three above differs. Served precomputed so no
    -- consumer has to remember to compare all three.
    method_changed      BOOLEAN     NOT NULL,

    computed_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_seller_run_deltas_network ON seller_run_deltas (network, computed_at DESC);
