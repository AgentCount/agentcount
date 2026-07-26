-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0007 — reshape `feedback` for the REAL ERC-8004 NewFeedback event.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- The deployed Reputation Registry (0x8004BAa1…) emits:
--   NewFeedback(uint256 agentId, address clientAddress, uint64 feedbackIndex,
--               int128 value, uint8 valueDecimals, string indexedTag1, ...)
--
-- i.e. feedback is left by a CLIENT ADDRESS about an agent, with a signed value
-- and its own decimal scale — NOT agent→agent with a small integer score, which
-- is what the placeholder schema assumed. We store what the contract actually
-- emits. The signed value is kept as text (int128 doesn't map cleanly to a bound
-- Rust numeric, and v1 only counts feedback rather than summing it).

-- The old from-side index and columns go; the new client/value columns arrive.
DROP INDEX IF EXISTS idx_feedback_from;
ALTER TABLE feedback DROP COLUMN IF EXISTS from_agent_id;
ALTER TABLE feedback DROP COLUMN IF EXISTS score;

ALTER TABLE feedback ADD COLUMN client_address TEXT     NOT NULL DEFAULT '';
ALTER TABLE feedback ADD COLUMN feedback_index BIGINT   NOT NULL DEFAULT 0;
ALTER TABLE feedback ADD COLUMN value          TEXT     NOT NULL DEFAULT '0';
ALTER TABLE feedback ADD COLUMN value_decimals SMALLINT NOT NULL DEFAULT 0;

-- Reciprocity work in the real model is address→agent, so index the client side.
CREATE INDEX idx_feedback_client ON feedback (chain, client_address);

-- Note on `agents`: the Identity Registry's `Registered` event carries an
-- `agentURI` (a metadata pointer), which we store in the existing `agents.domain`
-- column. The column name is now a slight misnomer — it holds a URI, not a bare
-- hostname. The enricher must be updated to fetch that URI directly rather than
-- constructing https://{domain}/.well-known/agent.json (tracked as a follow-up).
