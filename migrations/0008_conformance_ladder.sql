-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0008 — the conformance ladder: runs, snapshots, check results.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- The old model asked "is this agent alive?" and stored a mutable cache. This
-- one asks seven fixed questions per agent and stores the answers immutably,
-- per run, with the code and spec versions that produced them. A result you
-- cannot recompute is an opinion; a result you can is a fact.

-- One sweep. Everything else is keyed by this.
CREATE TABLE runs (
    run_id          UUID PRIMARY KEY,
    chain           TEXT NOT NULL,
    started_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at     TIMESTAMPTZ,
    -- Provenance. Every one of these is required to reproduce the run.
    schema_version  INT  NOT NULL,
    checker_version TEXT NOT NULL,   -- crate version, e.g. "0.2.0"
    checker_commit  TEXT NOT NULL,   -- git SHA of the sweeper build
    spec_commit     TEXT NOT NULL,   -- the ERC8004SPEC.md commit judged against
    rerun_command   TEXT NOT NULL,   -- the literal command that reproduces this
    agent_count     INT,             -- NULL until the sweep finishes
    FOREIGN KEY (chain) REFERENCES chains (chain)
);
CREATE INDEX idx_runs_chain_started ON runs (chain, started_at DESC);

-- What the chain said about one agent at one moment, read by CALL (ownerOf,
-- tokenURI) rather than inferred from a mint event — so a transferred NFT or
-- an updated URI is seen, not missed.
CREATE TABLE agent_snapshots (
    run_id       UUID   NOT NULL REFERENCES runs (run_id),
    chain        TEXT   NOT NULL,
    agent_id     BIGINT NOT NULL,
    token_id     NUMERIC NOT NULL,        -- ERC-721 token id; NUMERIC, uint256 exceeds i64
    owner        TEXT   NOT NULL,         -- lowercase hex, from ownerOf() at this block
    agent_uri    TEXT   NOT NULL,         -- from tokenURI(); '' is a legitimate value
    block_number BIGINT NOT NULL,         -- the block these reads were pinned to
    observed_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, chain, agent_id)
);
CREATE INDEX idx_snapshots_owner ON agent_snapshots (chain, owner);

-- One row per (run, agent, rung). A rung we did not run has NO ROW — absence
-- means "not checked", never "failed". That distinction is the product.
CREATE TABLE check_results (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_id     UUID   NOT NULL REFERENCES runs (run_id),
    chain      TEXT   NOT NULL,
    agent_id   BIGINT NOT NULL,
    rung       SMALLINT NOT NULL CHECK (rung BETWEEN 1 AND 7),
    name       TEXT   NOT NULL,       -- 'registered' | 'resolvable' | ...
    status     TEXT   NOT NULL CHECK (status IN ('pass','fail','skipped','error')),
    -- The proof, shaped per rung. Never prose.
    evidence   JSONB  NOT NULL,
    checked_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT check_results_unique UNIQUE (run_id, chain, agent_id, rung)
);
CREATE INDEX idx_check_results_lookup ON check_results (run_id, chain, agent_id);
-- Base rates are `GROUP BY rung, status` over one run: index that path.
CREATE INDEX idx_check_results_rates ON check_results (run_id, rung, status);

-- ── Retire the old observation model ────────────────────────────────────────
-- These measured availability, not conformance, and their archive is too thin
-- to re-judge (no content-type, no raw bytes). The decision to drop rather
-- than migrate was taken deliberately; see the Day 1 plan.
DROP TABLE IF EXISTS flag_events;
DROP TABLE IF EXISTS flags;
DROP TABLE IF EXISTS probe_history;
DROP TABLE IF EXISTS metadata_snapshots;
DROP TABLE IF EXISTS agent_enrichment;
DROP TABLE IF EXISTS payment_observations;
DROP TABLE IF EXISTS test_purchases;
ALTER TABLE agents DROP COLUMN IF EXISTS last_enriched_at;
