-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0006 — retire the scalar-score schema.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- The 0-100 trust score is dead: with no observable ground truth to calibrate
-- weights against, any weighting is aesthetic. The product is facts and flags
-- (migration 0005). This migration removes the score cache, the wipe-and-
-- replace cluster tables (superseded by flags/flag_events), the denormalised
-- suspicion column, and the never-written economic_activity table (superseded
-- by payment_observations, which carries tier + provenance).

DROP TABLE IF EXISTS scores;
DROP TABLE IF EXISTS cluster_members;
DROP TABLE IF EXISTS clusters;
DROP TABLE IF EXISTS economic_activity;
ALTER TABLE agents DROP COLUMN IF EXISTS suspicion;
