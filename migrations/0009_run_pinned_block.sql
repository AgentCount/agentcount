-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0009 — record the block a run was pinned to, on the run itself.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- `agent_snapshots.block_number` already carries this per row, but a resumable
-- sweep needs it on `runs` too: resuming means re-opening an EXISTING run and
-- reading the remaining agents at the SAME block the first session used, never
-- a fresher one — a run assembled across two different blocks would describe
-- a population that never simultaneously existed, which is the whole reason
-- pinning exists. Reading it off `runs` directly means resume never has to
-- infer the pinned block from row data (which would break for a run with zero
-- surviving snapshots).
ALTER TABLE runs ADD COLUMN pinned_block BIGINT;

-- Backfill the run that died mid-sweep on 2026-07-27 (agent 16791's tokenURI()
-- contains a NUL byte Postgres rejects — see the sweeper's NUL-escaping fix)
-- so it is resumable via SWEEP_RESUME.
--
-- NOTE: the incident report that requested this backfill named block
-- 49197387. That number does NOT match this run's own records: both
-- `runs.rerun_command` ("...# at block 49197467", written by the sweeper
-- itself when the run opened) and every one of the 16,789
-- `agent_snapshots.block_number` values already written by this run agree on
-- 49197467, and so does data/ead1a77f-31cd-40d3-aa52-4292dbb4d100/manifest.json
-- (written before the sweep began, from the same in-memory value). Backfilling
-- the number from the report instead of the run's own evidence would pin the
-- resumed remainder to a DIFFERENT block than the one the first 16,789 agents
-- were actually read at — precisely the two-blocks-in-one-run failure pinning
-- exists to prevent. Backfilling the value the run itself already recorded
-- three times over, 49197467, not 49197387.
UPDATE runs
SET pinned_block = 49197467
WHERE run_id = 'ead1a77f-31cd-40d3-aa52-4292dbb4d100';
