-- scripts/seed_chains.sql
-- Seed the chains the indexer should follow. Run manually:
--   psql "$DATABASE_URL" -f scripts/seed_chains.sql
--
-- These are the deployed ERC-8004 v1 registry addresses on Base (CREATE2, so the
-- same addresses appear on other chains too). No CANONICAL Validation Registry
-- was ever deployed by the ERC-8004 team — validation_registry is NULL, which
-- the code reads as "this feature is absent on this chain".
--
-- ⚠ 2026-07-30 — that NULL is right about the canonical deployment and WRONG as
-- a description of the chain. Third-party Validation Registries are deployed and
-- in use: 10 distinct addresses across Base and Celo, every one of them wired to
-- the canonical Identity Registry below, carrying 105 validation requests for 23
-- agents. BSC and mainnet really are at zero. Found by scanning the spec's event
-- topics with no address filter, precisely because filtering on this NULL would
-- have returned zero and confirmed the assumption. Full detail and the
-- disagreement this represents: analysis/validation-registry.md.
--
-- These addresses are NOT added to the column: they are third-party, mutually
-- unrelated, and there is no single "the" Validation Registry for a chain. A
-- one-address column cannot express ten, and picking one would be a judgement
-- the data does not support.

INSERT INTO chains (chain, chain_id, identity_registry, reputation_registry,
                    validation_registry, deploy_block, confirmations, enabled)
VALUES (
    'base', 8453,
    '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',  -- Identity Registry
    '0x8004baa17c55a88189ae136b182e5fda19de9b63',  -- Reputation Registry
    NULL,                                           -- see the Validation Registry note above
    -- The Identity Registry's actual creation block, replacing the 0 that made
    -- every backfill scan Base from genesis. This is NOT a guess — the old
    -- comment here warned against one, because a value that is too HIGH
    -- silently skips real registrations. It was found by binary search on
    -- `eth_getCode` and verified on both sides of the boundary: 0 bytes of code
    -- at 41,663,782, 130 bytes at 41,663,783. It cannot be too high, because
    -- the contract did not exist one block earlier.
    41663783,
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
-- `deploy_block` is now the Identity Registry's real creation block on each
-- chain, replacing the 0 that was the safe default while the exact value was
-- unknown. Each was found by binary search on `eth_getCode` and verified on
-- both sides of the boundary (0 bytes of code at deploy−1, 130 bytes at
-- deploy), so none can be too high — the header's warning against guessing a
-- too-high value still stands, and this is a measurement, not a guess:
--
--     base     41,663,783      mainnet  24,339,871
--     bsc      79,027,268      celo     58,396,724
--
-- robinhood stays 0: no agent has been minted there, so nothing has needed the
-- value and it has not been measured.
--
-- This costs empty ranges on a first LOG backfill only — agent enumeration
-- binary-searches `ownerOf` and never reads it.
--
-- `robinhood` is seeded DISABLED. Both registries are deployed there but no
-- agent has been minted, so a sweep would record an empty run. Flip `enabled`
-- when the first agent appears.

INSERT INTO chains (chain, chain_id, identity_registry, reputation_registry,
                    validation_registry, deploy_block, confirmations, enabled)
VALUES
    ('bsc',       56,    '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                         '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL, 79027268, 30, true),
    ('mainnet',   1,     '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                         '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL, 24339871, 30, true),
    ('celo',      42220, '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                         '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL, 58396724, 30, true),
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
