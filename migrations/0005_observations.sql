-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0005 — append-only observations: the longitudinal moat.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- Everything here is history we never rewrite. A competitor can copy the
-- harness in weeks; they cannot copy what we observed at the time.

-- Address identity backstop: the indexer writes lowercase, but a generated
-- column makes "joins never fragment on case" a database guarantee, not a
-- code convention.
ALTER TABLE agents ADD COLUMN address_norm TEXT
    GENERATED ALWAYS AS (lower(address)) STORED;
CREATE INDEX idx_agents_address ON agents (address_norm);

-- Observation history must survive everything; drop the cascade that would
-- let an agent deletion erase it. (agent_enrichment keeps its cascade — it's
-- a cache, not history.)
ALTER TABLE probe_history
    DROP CONSTRAINT IF EXISTS probe_history_chain_agent_id_fkey,
    DROP CONSTRAINT IF EXISTS probe_history_chain_fkey;
ALTER TABLE probe_history
    ADD CONSTRAINT probe_history_agent_fk
    FOREIGN KEY (chain, agent_id) REFERENCES agents (chain, agent_id);

-- Every metadata fetch, kept forever. Content rots (IPFS/HTTP); this table is
-- the archive of what a domain served at each point in time. Failures are data.
CREATE TABLE metadata_snapshots (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    chain        TEXT   NOT NULL,
    agent_id     BIGINT NOT NULL,
    url          TEXT   NOT NULL,
    fetched_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    http_status  INT,                          -- NULL when the request never completed
    content_hash TEXT,                         -- sha256 of the body; dedupe/change detection
    body         JSONB,                        -- NULL unless we got parseable JSON
    error        TEXT,                         -- outcome label / error detail on failure
    FOREIGN KEY (chain, agent_id) REFERENCES agents (chain, agent_id)
);
CREATE INDEX idx_snapshots_agent ON metadata_snapshots (chain, agent_id, fetched_at DESC);

-- Tiered payment evidence (thesis: verified settlements vs plausible payments).
-- NO WRITER this phase — the schema exists so month-two ingestion appends here
-- instead of forcing a migration under two months of history. Every row names
-- its tier and provenance.
CREATE TABLE payment_observations (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    chain        TEXT   NOT NULL,
    agent_id     BIGINT NOT NULL,
    direction    TEXT   NOT NULL,              -- 'inbound' | 'outbound'
    counterparty TEXT   NOT NULL,              -- lowercase hex
    token        TEXT   NOT NULL,              -- token contract, lowercase hex
    amount_raw   NUMERIC NOT NULL,             -- raw token units; integers, never floats
    tier         SMALLINT NOT NULL,            -- 1 = verified settlement, 2 = plausible payment
    provenance   TEXT   NOT NULL,              -- 'test_purchase' | 'x402_facilitator' | 'direct_transfer'
    tx_hash      TEXT   NOT NULL,
    block        BIGINT NOT NULL,
    observed_at  TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (chain, agent_id) REFERENCES agents (chain, agent_id),
    UNIQUE (chain, tx_hash, agent_id, counterparty, direction)
);
CREATE INDEX idx_payments_agent ON payment_observations (chain, agent_id, tier);

-- Test-purchase receipts. NO WRITER this phase — the shopper crate lands in
-- phase 2 and appends here. Tier-1 evidence: we were the counterparty, zero
-- interpretation.
CREATE TABLE test_purchases (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    chain           TEXT   NOT NULL,
    agent_id        BIGINT NOT NULL,
    prober_address  TEXT   NOT NULL,
    endpoint_url    TEXT   NOT NULL,
    amount_raw      NUMERIC NOT NULL,
    token           TEXT   NOT NULL,
    request         JSONB  NOT NULL,           -- what we sent (sanitized headers, body)
    response_status INT,
    response_body   JSONB,
    response_body_hash TEXT,
    tx_hash         TEXT,                      -- NULL if payment never settled
    outcome         TEXT   NOT NULL,           -- 'delivered'|'paid_no_delivery'|'refused'|'error'
    latency_ms      INT,
    started_at      TIMESTAMPTZ NOT NULL,
    settled_at      TIMESTAMPTZ,
    FOREIGN KEY (chain, agent_id) REFERENCES agents (chain, agent_id)
);

-- Flags: evidence-backed claims of coordination or decay. One current row per
-- (subject, kind); ALL state changes flow through flag_events, append-only.
-- Replaces the wipe-and-replace clusters tables (dropped in migration 0006).
CREATE TABLE flags (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    chain      TEXT   NOT NULL,
    agent_id   BIGINT NOT NULL,
    kind       TEXT   NOT NULL,                -- 'shared_operator' | 'synchronized_registration' | 'reciprocal_feedback'
    evidence   JSONB  NOT NULL,                -- peers, addresses, windows, tx refs — the proof
    raised_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (chain, agent_id) REFERENCES agents (chain, agent_id),
    CONSTRAINT flags_subject_kind_unique UNIQUE (chain, agent_id, kind)
);
CREATE TABLE flag_events (
    id       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    flag_id  BIGINT NOT NULL REFERENCES flags (id),
    event    TEXT   NOT NULL,                  -- 'raised' | 'evidence_added' | 'cleared'
    detail   JSONB,
    at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_flag_events_flag ON flag_events (flag_id, at DESC);
