-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0001 — core chain data (written by the `indexer`)
-- ─────────────────────────────────────────────────────────────────────────────
--
-- HOW MIGRATIONS WORK: each file in migrations/ is applied once, in filename
-- order, by `sqlx migrate run`. sqlx records which have run in a table called
-- `_sqlx_migrations`, so re-running is safe — already-applied files are skipped.
-- The numeric prefix (0001, 0002, …) is what defines the order, so never
-- renumber a file that has already been applied in any environment.
--
-- Rule of thumb: migrations are append-only history. To change the schema you
-- add a NEW migration; you don't edit an old one.
--
-- These tables are the raw truth the indexer writes. Everything else in the
-- system is derived from them.

-- The agents themselves. One row per ERC-8004 identity, per chain.
CREATE TABLE agents (
    -- The on-chain agent id. A registry assigns these; they're unique per chain,
    -- so the PRIMARY KEY is the (chain, agent_id) PAIR, not agent_id alone.
    chain            TEXT   NOT NULL,          -- 'ethereum' | 'base'
    agent_id         BIGINT NOT NULL,          -- the ERC-8004 id
    -- The wallet address that controls the agent (hex string, lower-cased).
    address          TEXT   NOT NULL,
    -- The domain the agent registered — where its agent-card is hosted.
    domain           TEXT   NOT NULL,
    -- Provenance: which block/tx first registered this agent. Lets us compute
    -- "age" and audit back to the source event.
    registered_block BIGINT NOT NULL,
    registered_at    TIMESTAMPTZ NOT NULL,     -- block timestamp
    registered_tx    TEXT   NOT NULL,          -- tx hash, for auditing
    -- Bookkeeping for the enricher: when did we last refresh off-chain data?
    -- NULL means "never enriched yet". The enricher's query keys off this.
    last_enriched_at TIMESTAMPTZ,
    -- The enricher's clustering verdict, denormalised onto the agent so the
    -- scorer can read it in one lookup. 0 = looks organic, 1 = looks coordinated.
    suspicion        DOUBLE PRECISION NOT NULL DEFAULT 0,

    PRIMARY KEY (chain, agent_id)
);

-- An append-only audit log of every raw registry event we decoded. We keep this
-- even though we also write typed rows (feedback, validations, …) because the
-- raw log is the ground truth: if a decoding bug is found later, we can reprocess
-- from here without re-scanning the chain.
CREATE TABLE raw_events (
    -- `GENERATED ALWAYS AS IDENTITY` is the modern SQL-standard auto-increment
    -- (preferred over the old `SERIAL`). Postgres assigns the id for us.
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    chain       TEXT   NOT NULL,
    contract    TEXT   NOT NULL,               -- which registry emitted it
    event_name  TEXT   NOT NULL,               -- 'AgentRegistered', etc.
    block       BIGINT NOT NULL,
    tx_hash     TEXT   NOT NULL,
    log_index   INT    NOT NULL,               -- position of the log within the tx
    payload     JSONB  NOT NULL,               -- the decoded fields, as JSON
    ingested_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- (chain, tx_hash, log_index) uniquely identifies a log, so this UNIQUE
    -- constraint makes re-ingesting the same block idempotent: a duplicate insert
    -- fails harmlessly instead of double-counting.
    UNIQUE (chain, tx_hash, log_index)
);

-- Feedback attestations from the Reputation Registry: one agent rating another.
-- This is the raw material the scoring crate's reputation sub-score chews on.
CREATE TABLE feedback (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    chain         TEXT   NOT NULL,
    from_agent_id BIGINT NOT NULL,             -- who gave the feedback
    to_agent_id   BIGINT NOT NULL,             -- who it's about
    score         SMALLINT NOT NULL,           -- raw attested value
    block         BIGINT NOT NULL,
    tx_hash       TEXT   NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL,
    UNIQUE (chain, tx_hash, from_agent_id, to_agent_id)
);

-- Indexes matter as soon as you have data. The scorer looks up all feedback
-- pointing AT an agent, so index the target column.
CREATE INDEX idx_feedback_to ON feedback (chain, to_agent_id);
-- The clustering stage walks the feedback graph from the source side, so index
-- that direction too. (Reciprocity checks need both.)
CREATE INDEX idx_feedback_from ON feedback (chain, from_agent_id);

-- Validation outcomes from the Validation Registry: a validator attesting to
-- whether some work was correct.
CREATE TABLE validations (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    chain        TEXT   NOT NULL,
    validator_id BIGINT NOT NULL,
    subject_id   BIGINT NOT NULL,              -- the agent whose work was validated
    passed       BOOLEAN NOT NULL,
    block        BIGINT NOT NULL,
    tx_hash      TEXT   NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL,
    UNIQUE (chain, tx_hash, validator_id, subject_id)
);

-- The indexer's resume point. One row per chain (and, if you split them, per
-- contract). `last_block` is the highest block we've FULLY processed; on restart
-- the indexer reads this and continues from the next block. This tiny table is
-- what makes the indexer crash-safe.
CREATE TABLE indexer_cursor (
    chain      TEXT   NOT NULL PRIMARY KEY,
    last_block BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
