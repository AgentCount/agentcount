-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0013 — capture the MINTER: who sent the registration transaction.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- "The agent" is at least eight different parties wearing one word, and this
-- census reads three of them. The minter — whoever called `register()` — is one
-- of the ones it could read and did not. It is frequently NOT the first owner:
-- a platform registering on a customer's behalf is the ordinary case, and on
-- Celo two addresses minted 87.9% of the chain.
--
-- Not storing it has already cost real work. Where a published report names a
-- minter it was pulled by hand from the mint transaction and says so, and the
-- role glossary has to carry "not read" against a field the chain hands over
-- for free.
--
-- ADDITIVE AND NULLABLE, deliberately. Every existing row keeps its meaning:
-- NULL here means "this run predates minter capture", never "no minter".
-- Backfill is a separate job; new sweeps populate it.
--
-- Why three columns rather than one: the minter is an inference from the
-- registration transaction, and the transaction is the evidence for it. Storing
-- the tx hash and block alongside means a reader can re-derive the minter
-- without trusting our read — which is the same standard every other claim in
-- this schema is held to.

ALTER TABLE agent_snapshots
    ADD COLUMN minter               TEXT,
    ADD COLUMN registration_tx_hash TEXT,
    ADD COLUMN registration_block   BIGINT;

COMMENT ON COLUMN agent_snapshots.minter IS
    'Sender of the registration transaction (the `from` of the tx that emitted '
    'Registered). NOT the same role as `owner`: a platform minting on a '
    'customer''s behalf is the common case. NULL = not captured by this run.';

COMMENT ON COLUMN agent_snapshots.registration_tx_hash IS
    'Transaction that emitted this agent''s Registered event — the evidence '
    'behind `minter`, so the claim is re-derivable without trusting our read.';

COMMENT ON COLUMN agent_snapshots.registration_block IS
    'Block of the registration transaction. Also the lower bound of the '
    'agent''s existence — the correction that removed 82.5% of Base''s '
    'apparent payment value (see analysis/payments-corrections-ledger.md, '
    'PAY-2) needed exactly this and had to compute it out-of-band.';

-- Concentration questions ("how many agents did one address mint?") are the
-- first thing anyone asks of this column, on a table with 354,858 rows.
CREATE INDEX idx_agent_snapshots_minter
    ON agent_snapshots (chain, minter)
    WHERE minter IS NOT NULL;
