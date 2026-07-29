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

-- ─────────────────────────────────────────────────────────────────────────────
-- 2026-07-29 — the other chains the registries are deployed on.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- The CREATE2 claim in the header was verified rather than assumed: `eth_getCode`
-- against both registry addresses returns code on BNB Chain, Ethereum mainnet,
-- Celo and Robinhood Chain. Agent populations were then counted with the same
-- contiguous-id binary search `crates/chain` uses (`ownerOf`, not `eth_getLogs`,
-- since `totalSupply()` reverts on this contract), read live on 2026-07-29:
--
--     base       chain_id 8453    60,129 agents   (swept)
--     bsc        chain_id 56     244,258 agents   (NOT swept — 4x Base)
--     mainnet    chain_id 1       40,729 agents   (NOT swept)
--     celo       chain_id 42220    9,747 agents   (NOT swept)
--     robinhood  chain_id 4663          0 agents   (nothing to sweep yet)
--
-- So Base, the only chain this census has published, is roughly 17% of the
-- registered ERC-8004 population.
--
-- `chain` must match the RPC env var: `crates/indexer`'s `rpc_env_var()` builds
-- `RPC_URL_{CHAIN.to_uppercase()}`, so the row named `mainnet` reads
-- RPC_URL_MAINNET. The indexer independently checks the RPC's `eth_chainId`
-- against `chain_id` below, which is what stops a mis-set URL from being swept
-- as the wrong chain.
--
-- `deploy_block` is 0 for every row, and that is the safe default the header
-- above argues for: too high silently skips real registrations. It costs empty
-- ranges on a first LOG backfill only — agent enumeration binary-searches
-- `ownerOf` and never reads it.
--
-- `robinhood` is seeded DISABLED. Both registries are deployed there but no
-- agent has been minted, so a sweep would record an empty run. Flip `enabled`
-- when the first agent appears.

INSERT INTO chains (chain, chain_id, identity_registry, reputation_registry,
                    validation_registry, deploy_block, confirmations, enabled)
VALUES
    ('bsc',       56,    '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                         '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL, 0, 30, true),
    ('mainnet',   1,     '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                         '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL, 0, 30, true),
    ('celo',      42220, '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                         '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL, 0, 30, true),
    ('robinhood', 4663,  '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                         '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL, 0, 30, false)
ON CONFLICT (chain) DO UPDATE SET
    chain_id            = EXCLUDED.chain_id,
    identity_registry   = EXCLUDED.identity_registry,
    reputation_registry = EXCLUDED.reputation_registry,
    validation_registry = EXCLUDED.validation_registry,
    deploy_block        = EXCLUDED.deploy_block,
    confirmations       = EXCLUDED.confirmations,
    enabled             = EXCLUDED.enabled;
