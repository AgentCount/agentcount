-- scripts/seed_chains.sql
-- Seed the chains the indexer should follow. Run manually:
--   psql "$DATABASE_URL" -f scripts/seed_chains.sql
--
-- ⚠ FOUNDER INPUT REQUIRED: replace the zero addresses below with the real
-- ERC-8004 registry addresses (CREATE2 → identical across chains) and the real
-- Base deploy block. The indexer REFUSES to run a chain whose identity_registry
-- is the zero address, so a forgotten edit fails loudly instead of indexing
-- nothing. Verify addresses against the deployed registries before seeding.

INSERT INTO chains (chain, chain_id, identity_registry, reputation_registry,
                    validation_registry, deploy_block, confirmations, enabled)
VALUES (
    'base', 8453,
    '0x0000000000000000000000000000000000000000',  -- TODO(founder): Identity Registry
    '0x0000000000000000000000000000000000000000',  -- TODO(founder): Reputation Registry (or NULL)
    '0x0000000000000000000000000000000000000000',  -- TODO(founder): Validation Registry (or NULL)
    0,                                              -- TODO(founder): registry deploy block on Base
    30, true
)
ON CONFLICT (chain) DO UPDATE SET
    identity_registry   = EXCLUDED.identity_registry,
    reputation_registry = EXCLUDED.reputation_registry,
    validation_registry = EXCLUDED.validation_registry,
    deploy_block        = EXCLUDED.deploy_block,
    confirmations       = EXCLUDED.confirmations,
    enabled             = EXCLUDED.enabled;
