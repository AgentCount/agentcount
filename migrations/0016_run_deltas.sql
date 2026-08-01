-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0016 — week-over-week deltas between two runs on one chain.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- A census taken once is a photograph. Taken weekly it becomes the only thing
-- in this ecosystem that can answer "what changed" — and one of those answers
-- is not available from any other source:
--
--   **agents that STOPPED resolving.**
--
-- Registration counts go up and are published by everyone. Nobody publishes
-- decay, because computing it requires having asked the same question of the
-- same population at two pinned blocks and kept both answers. This project
-- does, so this table exists to make that number a first-class artifact rather
-- than something recomputed differently each time somebody asks.
--
-- ## Why a table and not a view
--
-- Two reasons, both about what a delta IS.
--
-- It is a claim about a specific PAIR of runs, and the pair is chosen once. A
-- view would silently re-pick "the previous run" as new runs land, so a delta
-- someone cited last week would quietly become a different number. Written
-- once, it stays what it was.
--
-- And it is expensive: comparing two 244,208-agent runs is a full join over
-- both, which is not something a page load should do. Every figure here is
-- computed by the `delta` binary after a sweep finishes and read back as rows.
CREATE TABLE IF NOT EXISTS run_deltas (
    -- The newer run. One delta per run, replacing any earlier computation of
    -- it — a delta is derived, so recomputing it after a fix is legitimate in
    -- a way that rewriting a run's own results never is.
    run_id              uuid        PRIMARY KEY REFERENCES runs(run_id) ON DELETE CASCADE,
    -- What it is compared against. NULL is impossible: a run with no
    -- predecessor gets no delta row at all, because "first observation" and
    -- "nothing changed" are different claims and a row of zeroes would read
    -- as the second.
    previous_run_id     uuid        NOT NULL REFERENCES runs(run_id) ON DELETE CASCADE,
    chain               text        NOT NULL,

    -- Population.
    agents_before       integer     NOT NULL,
    agents_after        integer     NOT NULL,
    -- Present in the newer run, absent from the older one.
    newly_registered    integer     NOT NULL,
    -- Present in the older run, absent from the newer one. Expected to be 0 —
    -- an ERC-721 is not usually burned — so a non-zero value is a finding.
    disappeared         integer     NOT NULL,

    -- Rung 2 (`resolvable`), the reachability question, called out from the
    -- per-rung flips below because it is the one with a published series.
    newly_resolving     integer     NOT NULL,
    stopped_resolving   integer     NOT NULL,

    -- Every rung's status transitions, as
    --   [{"rung":2,"from":"pass","to":"fail","agents":12}, …]
    -- Only rungs and pairs that actually moved appear. An agent present in one
    -- run and not the other is NOT a transition and is counted above instead —
    -- folding "arrived" into "changed status" would make every sweep's largest
    -- flip be new registrations.
    flips               jsonb       NOT NULL,

    -- ── The confound, recorded rather than assumed away ──────────────────
    --
    -- A delta is only a statement about the WORLD if both runs asked the same
    -- questions. When the checker changed between them, some agents moved
    -- because we changed, not because they did — and the two are
    -- indistinguishable from the flip counts alone.
    --
    -- The first delta computed on real data showed this immediately: between
    -- the 2026-07-28 and 2026-07-29 Base runs, 564 agents "stopped resolving"
    -- in a single day. Those runs also straddle P0 FIX 6, so an unknown share
    -- of the 564 is method. Publishing that number as decay would have been
    -- the most quotable wrong thing this project has produced.
    --
    -- So both versions are stored, and any surface that renders a delta must
    -- say when they differ. A weekly schedule normally produces equal values
    -- here, which is exactly when the delta means what it appears to mean.
    checker_before      text        NOT NULL,
    checker_after       text        NOT NULL,
    schema_before       integer     NOT NULL,
    schema_after        integer     NOT NULL,

    computed_at         timestamptz NOT NULL DEFAULT now()
);

COMMENT ON COLUMN run_deltas.checker_before IS
    'When this differs from checker_after, some flips are method changes rather than changes in the world. Any published figure must say so.';

CREATE INDEX IF NOT EXISTS run_deltas_chain_idx ON run_deltas (chain, computed_at DESC);
