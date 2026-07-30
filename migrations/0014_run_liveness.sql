-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0014 — a run must be able to say it died.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- The moat is an unbroken run history, and the worst failure mode this project
-- has is a run that fails while looking healthy. It happened: an analysis scan
-- reached 16,000 of 27,108 calls, the machine slept, its sockets died, and the
-- process sat at 0% CPU for three hours. Nothing errored. Nothing logged. From
-- the outside it was indistinguishable from slow progress.
--
-- Until now a sweep could end the same way and leave the same trace: a `runs`
-- row with `finished_at IS NULL`. That is ambiguous between the two states a
-- reader most needs to tell apart —
--
--     "this run is in progress"   and   "this run died and nobody noticed"
--
-- and the ambiguity resolves the wrong way by default, because a dead run and
-- a running one look identical until someone thinks to check the clock.
--
-- `status` makes the lifecycle explicit and `last_progress_at` makes a stall
-- detectable without watching the process. A gap in the history is now always
-- visible in the history itself.

ALTER TABLE runs
    ADD COLUMN status           TEXT NOT NULL DEFAULT 'finished',
    ADD COLUMN last_progress_at TIMESTAMPTZ,
    ADD COLUMN failure_reason   TEXT;

-- Existing rows: every run already in this table either completed or was
-- abandoned before this column existed. Defaulting to 'finished' would lie
-- about the abandoned ones, so the ones without `finished_at` are corrected to
-- 'unknown' — a state that means exactly "this run predates liveness tracking
-- and we cannot say", which is the honest answer and not the same as 'failed'.
UPDATE runs SET status = 'unknown' WHERE finished_at IS NULL;

ALTER TABLE runs
    ADD CONSTRAINT runs_status_check
    CHECK (status IN ('running', 'finished', 'stalled', 'failed', 'unknown'));

COMMENT ON COLUMN runs.status IS
    'Lifecycle. running = in progress; finished = completed normally; '
    'stalled = the watchdog saw no progress for the stall timeout and killed '
    'it; failed = ended on an error; unknown = predates liveness tracking. '
    'A run that is not `finished` must never be quoted as a census.';

COMMENT ON COLUMN runs.last_progress_at IS
    'Heartbeat: last time this run wrote an agent. Lets a stalled run be '
    'spotted from the database alone, without watching the process.';

COMMENT ON COLUMN runs.failure_reason IS
    'Why a stalled or failed run ended, in one line. NULL for finished runs.';

-- "Show me every run that is not a clean census" is the query this exists for.
CREATE INDEX idx_runs_unfinished ON runs (chain, status) WHERE status <> 'finished';
