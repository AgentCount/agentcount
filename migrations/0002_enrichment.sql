-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0002 — enrichment & clustering (written by the `enricher`)
-- ─────────────────────────────────────────────────────────────────────────────
--
-- These tables hold everything the enricher derives: fetched metadata, the
-- history of liveness probes, observed economic activity, and the Sybil clusters.

-- One row per agent holding its latest off-chain snapshot. This is a 1:1
-- companion to `agents`, kept separate so the indexer and enricher never write
-- the same table (clean ownership: each writer owns its tables).
CREATE TABLE agent_enrichment (
    chain            TEXT NOT NULL,
    agent_id         BIGINT NOT NULL,
    -- The parsed agent-card, kept as JSONB so we don't have to model every field.
    -- NULL if we never got a valid card.
    agent_card       JSONB,
    -- Was the endpoint healthy at the most recent probe? A quick flag for the UI;
    -- the *rate* over time comes from probe_history below.
    endpoint_healthy BOOLEAN NOT NULL DEFAULT false,
    last_probed_at   TIMESTAMPTZ,
    -- Foreign key ties enrichment to a real agent, and ON DELETE CASCADE means
    -- deleting an agent cleans up its enrichment automatically.
    PRIMARY KEY (chain, agent_id),
    FOREIGN KEY (chain, agent_id) REFERENCES agents (chain, agent_id) ON DELETE CASCADE
);

-- The full history of liveness probes, one row per probe. The scoring crate's
-- liveness sub-score is (roughly) the success rate over these rows, so we keep
-- them all rather than just the latest — a single lucky success shouldn't look
-- like perfect uptime.
CREATE TABLE probe_history (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    chain      TEXT NOT NULL,
    agent_id   BIGINT NOT NULL,
    -- Mirrors the `ProbeOutcome` enum in enricher/src/liveness.rs. Storing the
    -- outcome as text keeps the history human-readable and future-proof.
    outcome    TEXT NOT NULL,               -- 'healthy' | 'timeout' | 'unreachable' | ...
    latency_ms INT,                         -- NULL unless healthy
    probed_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (chain, agent_id) REFERENCES agents (chain, agent_id) ON DELETE CASCADE
);
CREATE INDEX idx_probe_history_agent ON probe_history (chain, agent_id, probed_at DESC);

-- Observed economic activity — the raw material for the payment sub-score.
-- "Payment" here is deliberately broad; decide precisely what you count (native
-- transfers, ERC-20, x402 receipts…) and document it. `counterparty` is the
-- other side of the transaction; counterparty DIVERSITY is the anti-gaming
-- signal the scorer cares about most.
CREATE TABLE economic_activity (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    chain        TEXT NOT NULL,
    agent_id     BIGINT NOT NULL,           -- the agent receiving/making payment
    counterparty TEXT NOT NULL,             -- the address on the other side
    value        NUMERIC NOT NULL,          -- normalized value; NUMERIC avoids float rounding
    token        TEXT,                      -- NULL = native coin
    tx_hash      TEXT NOT NULL,
    block        BIGINT NOT NULL,
    occurred_at  TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (chain, agent_id) REFERENCES agents (chain, agent_id) ON DELETE CASCADE,
    UNIQUE (chain, tx_hash, agent_id, counterparty)
);
CREATE INDEX idx_econ_agent ON economic_activity (chain, agent_id);

-- The clusters detected by enricher/src/clustering.rs. One row per cluster.
CREATE TABLE clusters (
    -- A UUID rather than a serial, so a cluster keeps a stable id even as the
    -- clustering is recomputed and re-inserted. (Postgres can generate these with
    -- gen_random_uuid() from the built-in pgcrypto/uuid support.)
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- The [0,1] coordination signal that becomes each member's `suspicion`.
    suspicion  DOUBLE PRECISION NOT NULL,
    -- Which heuristics flagged this cluster, as a JSON array of reason strings —
    -- shown on the explorer so the methodology is transparent.
    reasons    JSONB NOT NULL,
    detected_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Which agents belong to which cluster (many agents per cluster). The pair is
-- the primary key so an agent can't be listed in the same cluster twice.
CREATE TABLE cluster_members (
    cluster_id UUID   NOT NULL REFERENCES clusters (id) ON DELETE CASCADE,
    chain      TEXT   NOT NULL,
    agent_id   BIGINT NOT NULL,
    PRIMARY KEY (cluster_id, chain, agent_id),
    FOREIGN KEY (chain, agent_id) REFERENCES agents (chain, agent_id) ON DELETE CASCADE
);
