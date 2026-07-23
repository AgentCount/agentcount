-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0003 — computed trust scores (written by the `api`, or a batch job)
-- ─────────────────────────────────────────────────────────────────────────────
--
-- The `scoring` crate is a pure function, but we still persist its output so the
-- explorer's leaderboard can sort thousands of agents without recomputing on
-- every page load. Think of this table as a cache of the latest score per agent,
-- plus a little history so you can chart how scores move over time.

CREATE TABLE scores (
    id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    chain          TEXT NOT NULL,
    agent_id       BIGINT NOT NULL,
    -- The four positive sub-scores and the penalty, stored individually so the
    -- detail page can show the full breakdown, and so you can audit exactly how a
    -- final score was reached. All in [0, 1].
    payment        DOUBLE PRECISION NOT NULL,
    liveness       DOUBLE PRECISION NOT NULL,
    age            DOUBLE PRECISION NOT NULL,
    reputation     DOUBLE PRECISION NOT NULL,
    sybil_penalty  DOUBLE PRECISION NOT NULL,
    -- The single number the leaderboard sorts by: raw * (1 - sybil_penalty).
    final_score    DOUBLE PRECISION NOT NULL,
    computed_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (chain, agent_id) REFERENCES agents (chain, agent_id) ON DELETE CASCADE
);

-- We frequently want "the latest score for each agent" and "the top N agents by
-- score". This index serves both: newest-first per agent.
CREATE INDEX idx_scores_agent_latest ON scores (chain, agent_id, computed_at DESC);
-- And a plain index on final_score for the leaderboard ordering.
CREATE INDEX idx_scores_final ON scores (final_score DESC);

-- ── A note on "latest score per agent" ──────────────────────────────────────
-- Because `scores` keeps history, the leaderboard query needs the most recent
-- row per agent. Postgres's `DISTINCT ON` is the idiomatic tool:
--
--     SELECT DISTINCT ON (chain, agent_id) *
--     FROM scores
--     ORDER BY chain, agent_id, computed_at DESC;
--
-- When this table grows large, promote that query into a materialized view and
-- refresh it on a schedule — but don't optimise before you have the data to
-- justify it.
