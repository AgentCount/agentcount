-- scripts/seed_chains.sql
-- Seed the chains the indexer should follow. Run manually:
--   psql "$DATABASE_URL" -f scripts/seed_chains.sql
--
-- This file is IDEMPOTENT and CORRECTIVE. Every statement is
-- INSERT … ON CONFLICT (chain) DO UPDATE, so re-running it against a database
-- that already has rows overwrites the columns below rather than failing. That
-- is not incidental: production predates the deploy_block measurement and
-- carries deploy_block = 0 on every row, including base. Re-running this file
-- is how those zeros get fixed.
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
-- registered ERC-8004 population. (2026-08-04: with eight more chains measured
-- below, that share is 14% — see the recount at the end of this file.)
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
-- This costs empty ranges on a first LOG backfill only — agent enumeration
-- binary-searches `ownerOf` and never reads it.
--
-- `robinhood` is seeded DISABLED. Both registries are deployed there but no
-- agent has been minted, so a sweep would record an empty run. Flip `enabled`
-- when the first agent appears.
--
-- ⚠ 2026-08-04 — robinhood's deploy_block was 0 in this file, on the reasoning
-- that nothing had needed the value so nothing had measured it. It has now been
-- measured with everything else (12,058,809, both sides verified) and is set
-- below. The row is still 0 agents and still disabled; the difference is that
-- the day it is enabled, its first backfill will not start at genesis.

-- ─────────────────────────────────────────────────────────────────────────────
-- 2026-08-04 — every chain we hold an RPC for, measured rather than assumed.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- WHY THIS RUN HAPPENED: production's `chains` table has deploy_block = 0 on
-- EVERY row — base included, because production predates the base measurement
-- recorded above. Every backfill on every chain has therefore been scanning
-- from genesis. Fixing that meant re-deriving base to prove the method still
-- reproduces (it does, to the block) and deriving the rest the same way.
--
-- HOW: `scripts/derive-deploy-blocks.mjs`, in this repo, plain Node and zero
-- dependencies. It reads RPC URLs from the environment (RPC_URL_BASE, …) and
-- contains no secrets. Run it with:
--
--     for c in mainnet base bsc celo robinhood worldchain arbitrum op \
--              polygon megaeth xlayer monad gnosis; do
--       export RPC_URL_$(echo $c | tr a-z A-Z)=$(gcloud secrets versions access \
--         latest --secret=rpc-url-$c --project vippalo)
--     done
--     export RPC_URL_BILLIONS=https://rpc.billions.gateway.fm   # not on Alchemy
--     node scripts/derive-deploy-blocks.mjs --out /tmp/deploy-blocks.json
--
-- The whole file below is that script's output, transcribed. It was run twice;
-- every deploy_block and every agent count was byte-identical across the two
-- runs, which is the only reason they are written down as facts.
--
-- THE ASSERTION IS THE POINT. For each chain the script asserts BOTH SIDES of
-- the boundary before it will emit a number:
--
--     eth_getCode(identity, deploy_block − 1) == "0x"   (0 bytes: absent)
--     eth_getCode(identity, deploy_block)     != "0x"   (130 bytes: present)
--
-- Every row below passed both. That asymmetry is why the check exists: a
-- deploy_block that is too LOW only wastes scan time, but one that is too HIGH
-- silently skips real registrations and nothing anywhere complains. A value
-- that is code-free one block earlier cannot be too high. Any chain that
-- failed the assertion emits NO row — a missing chain is a visible gap, a
-- wrong deploy_block is an invisible one.
--
-- The identity registry returned exactly 130 bytes of code on all twelve
-- chains below, which is the CREATE2 claim in the header holding up: it is
-- byte-identical everywhere, not merely present.
--
-- `reputation_registry` is OBSERVED per chain, not copied from a neighbouring
-- row: eth_getCode was run against 0x8004baa1… separately on each chain. It
-- returned code on all twelve, so all twelve carry the address. The column is
-- NULL only where the contract genuinely is not there — which, on this set, is
-- nowhere.
--
-- ENABLED IS A MAINTAINER DECISION, NOT A CONSEQUENCE OF THIS FILE.
-- Every chain added on 2026-08-04 is seeded `enabled = false`. Only base, bsc,
-- celo and mainnet stay `enabled = true`, exactly as before this change.
-- Adding a row records that a chain EXISTS and where its data starts; it must
-- never change what actually gets swept. Sweeping a chain is a deliberate act:
-- flip `enabled` to true here AND add the chain to SWEEP_CHAINS. Two switches,
-- both explicit, because a census that silently grew its own population
-- between runs would make every published figure incomparable with the last.
--
-- Each row carries its agent count as of 2026-08-04 so a reader can see the
-- size of what they would be turning on. Those counts are a snapshot, not a
-- constraint — they are comments, and the sweep recounts from the chain.
--
-- CHAINS DELIBERATELY ABSENT FROM THIS FILE:
--
--   worldchain (chain_id 480) — NOT DEPLOYED. eth_getCode returns "0x" for
--     both the Identity and the Reputation Registry. There is nothing to
--     index; a row would be a claim that the registry is there.
--
--   monad (chain_id 143) — DEPLOYED, and deliberately NOT given a row.
--     The registry is live (130 bytes) and holds 10,189 agents, but the
--     deploy_block could not be determined: ARCHIVE DEPTH INSUFFICIENT. The
--     RPC serves state only ~390,000 blocks back, and at ~0.3s blocks that is
--     roughly the last 32 hours; the registry still has code at the oldest
--     height the node will answer, so the deploy is somewhere beyond the
--     window and the binary search cannot reach it. Guessing a block here is
--     exactly the failure this file exists to prevent, and seeding 0 would
--     mean a genesis scan of a ~93M-block chain. To add monad: re-run
--     derive-deploy-blocks.mjs with RPC_URL_MONAD pointed at a full archive
--     node, and paste the verified number in.
--
-- Coverage recount, 2026-08-04. Registered agents on every chain measured
-- here, including monad, total 433,951. Base — still the only chain this
-- census publishes — is 60,559 of them, about 14%. The 17% written above on
-- 2026-07-29 was not wrong then; it was computed over five chains because five
-- chains were all that had been looked at. Eight more chains later the
-- denominator grew and Base's share fell. Both numbers are kept, because the
-- gap between them is the finding.

INSERT INTO chains (chain, chain_id, identity_registry, reputation_registry,
                    validation_registry, deploy_block, confirmations, enabled)
VALUES
    -- ── Swept today. enabled = true, unchanged by this file. ────────────────
    ('bsc',        56,     '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                           '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL,  79027268, 30, true),
                           -- 250,814 agents. 0 bytes at 79,027,267 / 130 at 79,027,268.
    ('mainnet',    1,      '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                           '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL,  24339871, 30, true),
                           --  46,982 agents. 0 bytes at 24,339,870 / 130 at 24,339,871.
    ('celo',       42220,  '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                           '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL,  58396724, 30, true),
                           --   9,757 agents. 0 bytes at 58,396,723 / 130 at 58,396,724.

    -- ── Not swept. enabled = false. Flip this AND set SWEEP_CHAINS to sweep. ─
    ('billions',   45056,  '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                           '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL,   4915296, 30, false),
                           --  25,974 agents. 0 bytes at 4,915,295 / 130 at 4,915,296.
                           -- The only chain here not reachable via Alchemy:
                           -- RPC_URL_BILLIONS=https://rpc.billions.gateway.fm (no key).
    ('megaeth',    4326,   '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                           '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL,   7833805, 30, false),
                           --  12,727 agents. 0 bytes at 7,833,804 / 130 at 7,833,805.
    ('xlayer',     196,    '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                           '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL,  51947237, 30, false),
                           --  10,488 agents. 0 bytes at 51,947,236 / 130 at 51,947,237.
    ('gnosis',     100,    '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                           '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL,  44505010, 30, false),
                           --   4,106 agents. 0 bytes at 44,505,009 / 130 at 44,505,010.
    ('arbitrum',   42161,  '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                           '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL, 428895443, 30, false),
                           --   1,224 agents. 0 bytes at 428,895,442 / 130 at 428,895,443.
    ('polygon',    137,    '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                           '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL,  82458484, 30, false),
                           --     596 agents. 0 bytes at 82,458,483 / 130 at 82,458,484.
    ('op',         10,     '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                           '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL, 147514947, 30, false),
                           --     535 agents. 0 bytes at 147,514,946 / 130 at 147,514,947.
                           -- Named `op`, not `optimism`: the row name IS the env
                           -- var suffix, and the secret is rpc-url-op.
    ('robinhood',  4663,   '0x8004a169fb4a3325136eb29fa0ceb6d2e539a432',
                           '0x8004baa17c55a88189ae136b182e5fda19de9b63', NULL,  12058809, 30, false)
                           --       0 agents. 0 bytes at 12,058,808 / 130 at 12,058,809.
                           -- Registries deployed, nobody has minted. See the
                           -- 2026-08-04 note above for why this is no longer 0.
ON CONFLICT (chain) DO UPDATE SET
    chain_id            = EXCLUDED.chain_id,
    identity_registry   = EXCLUDED.identity_registry,
    reputation_registry = EXCLUDED.reputation_registry,
    validation_registry = EXCLUDED.validation_registry,
    deploy_block        = EXCLUDED.deploy_block,
    confirmations       = EXCLUDED.confirmations,
    enabled             = EXCLUDED.enabled;

-- `confirmations` stays at 30 on every row, including the fast chains. The
-- column comment in migrations/0004 notes that fast chains want more blocks for
-- the same wall-clock reorg buffer; none of the chains added today is swept, so
-- 30 is a placeholder that has never been exercised on them. Revisit it as part
-- of enabling a chain, not before — a number tuned for a sweep that has not
-- happened is another guess in a file that is trying not to contain any.
