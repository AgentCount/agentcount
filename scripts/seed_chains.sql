-- scripts/seed_chains.sql
-- Seed the chains the indexer should follow. Run manually:
--   psql "$DATABASE_URL" -f scripts/seed_chains.sql
--
-- These are the deployed ERC-8004 v1 registry addresses on Base (CREATE2, so the
-- same addresses appear on other chains too). Base has an Identity and a
-- Reputation registry but no Validation Registry — validation_registry is NULL,
-- which the code reads as "this feature is absent on this chain".

INSERT INTO chains (chain, chain_id, identity_registry, reputation_registry,
                    validation_registry, deploy_block, confirmations, enabled)
VALUES (
    'base', 8453,
    '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',  -- Identity Registry
    '0x8004baa17c55a88189ae136b182e5fda19de9b63',  -- Reputation Registry
    NULL,                                           -- no Validation Registry on Base
    -- ⚠ deploy_block 0 scans Base from genesis: SAFE (never misses a
    -- registration) but wastes ~1 hr / ~1M CU of empty ranges on the first
    -- backfill. Set this to the registry's actual creation block (BaseScan →
    -- the contract → "Contract Creation" tx → block number) to skip that.
    -- Do NOT guess a value that might be too HIGH — that would silently skip
    -- real registrations. Zero is the safe default until you have the exact block.
    0,
    30, true
)
ON CONFLICT (chain) DO UPDATE SET
    identity_registry   = EXCLUDED.identity_registry,
    reputation_registry = EXCLUDED.reputation_registry,
    validation_registry = EXCLUDED.validation_registry,
    deploy_block        = EXCLUDED.deploy_block,
    confirmations       = EXCLUDED.confirmations,
    enabled             = EXCLUDED.enabled;
