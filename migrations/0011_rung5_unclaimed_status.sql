-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0011 — rung 5 gains a fifth status: `unclaimed`.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- P0 FIX 3 (2026-07-29) reclassified `registrations` from MUST to SHOULD, so a
-- document can now pass rung 4 while carrying no `registrations` array at all.
-- Rung 5 (`bound`) then has nothing to verify, and none of the four original
-- statuses was honest for that case: `pass` would claim a verification that
-- never happened, `fail` would punish a merely-recommended field as hard as a
-- real on-chain mismatch, `skipped` would falsely imply an earlier rung
-- failed, and `error` would falsely imply this checker malfunctioned.
--
-- `unclaimed` names the case precisely: the agent made no binding claim for
-- rung 5 to check. See `crates/checks/src/model.rs` (`CheckStatus::Unclaimed`)
-- and `crates/checks/src/rung5_bound.rs` for the check logic, and
-- `METHODOLOGY.md` §2 for the published definition.
--
-- Postgres has no `ALTER CONSTRAINT`, so the old CHECK is dropped and a wider
-- one takes its place. This is additive only — every value the old constraint
-- accepted is still accepted, so no existing row is affected.
ALTER TABLE check_results DROP CONSTRAINT check_results_status_check;
ALTER TABLE check_results
    ADD CONSTRAINT check_results_status_check
    CHECK (status IN ('pass', 'fail', 'skipped', 'error', 'unclaimed'));
