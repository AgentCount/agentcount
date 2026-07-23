-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0004 — chains as data, and stronger on-chain provenance.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- Chains stop being hardcoded strings in the indexer and become rows here.
-- Adding a chain later is an INSERT, not a refactor. NULL registry columns
-- express per-chain feature variance (e.g. a chain without a Validation
-- Registry). Registry addresses are seeded separately (scripts/seed_chains.sql)
-- because they are data the founder supplies, not schema.

CREATE TABLE chains (
    chain               TEXT   PRIMARY KEY,     -- 'base', later 'ethereum', …
    chain_id            BIGINT NOT NULL,        -- EIP-155 id, e.g. 8453
    identity_registry   TEXT   NOT NULL,        -- lowercase hex; zero addr = "not configured yet"
    reputation_registry TEXT,                   -- NULL = registry absent on this chain
    validation_registry TEXT,                   -- NULL = registry absent on this chain
    deploy_block        BIGINT NOT NULL,        -- where backfill starts (replaces DEFAULT_START_BLOCK=0)
    confirmations       INT    NOT NULL DEFAULT 30, -- reorg buffer, per chain (fast chains need more blocks)
    enabled             BOOLEAN NOT NULL DEFAULT true
);

-- One tx can emit two feedback events for the same (from, to) pair; the old
-- unique key would silently drop the second. (chain, tx_hash, log_index)
-- uniquely identifies a log — same rule raw_events already uses.
ALTER TABLE feedback ADD COLUMN log_index INT NOT NULL DEFAULT 0;
ALTER TABLE feedback DROP CONSTRAINT IF EXISTS feedback_chain_tx_hash_from_agent_id_to_agent_id_key;
ALTER TABLE feedback ADD CONSTRAINT feedback_event_unique UNIQUE (chain, tx_hash, log_index);

ALTER TABLE validations ADD COLUMN log_index INT NOT NULL DEFAULT 0;
ALTER TABLE validations DROP CONSTRAINT IF EXISTS validations_chain_tx_hash_validator_id_subject_id_key;
ALTER TABLE validations ADD CONSTRAINT validations_event_unique UNIQUE (chain, tx_hash, log_index);

-- Reorg *detectability*: with the block hash stored, a reorged block can be
-- recognised and its range re-processed from the audit log. Nullable because
-- rows ingested before this migration have no hash.
ALTER TABLE raw_events ADD COLUMN block_hash TEXT;
