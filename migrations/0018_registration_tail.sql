-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0018 — the continuous registration tail.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- Continuous registration tail: agents seen on-chain since the last census run.
--
-- ## The boundary, and why it is structural rather than a WHERE clause
--
-- Every table this census publishes from is run-scoped: `agent_snapshots` is
-- keyed (run_id, chain, agent_id), `check_results` is keyed by run, and every
-- rate, finding and archive is computed by joining on a run. A census figure
-- is therefore, by construction, a statement about one pinned block.
--
-- This table is NOT run-scoped: it has no run_id key, so no census query
-- reaches it. That is the whole safety property. A query that sums census
-- data starts from `runs` and walks down through run-keyed tables, and this
-- table is not on that path, so tail rows cannot enter a published figure by
-- someone forgetting a filter.
--
-- The one reference to `runs` is `superseded_by_run`, which is nullable, is
-- written only to retire a row from the "unswept" view, and is never a term
-- in any rate, finding or archive. It points from tail to census and never
-- the other way: it records that a census has since covered this id, not
-- that this row belongs to that census. Enforcing the separation with a flag column on `agent_snapshots`
-- would have inverted that — every existing aggregate would have silently
-- started including unchecked agents until someone added `WHERE is_tail =
-- false` to all of them, and the one that got missed would be the one that
-- published a wrong number.
--
-- The cost of this choice is duplication: an agent discovered here and later
-- swept appears in both places. That is correct. They are different claims
-- about different moments — "seen at block N, unchecked" and "as it existed
-- at pinned block X, with seven answers" — and collapsing them into one row
-- would destroy the distinction the census sells.
--
-- ## What is stored, and what is deliberately not
--
-- Only what an on-chain read gives cheaply: the id, its owner, the URI it
-- declares, and the block that was read. No check results, ever. A tail row
-- is not a judgement about an agent; it is a receipt that the registry
-- contains an id we have not yet asked any question about.
CREATE TABLE registration_tail (
    chain            TEXT   NOT NULL,
    agent_id         BIGINT NOT NULL,
    token_id         NUMERIC NOT NULL,
    owner            TEXT   NOT NULL,          -- lowercase hex, ownerOf() at discovery_block
    agent_uri        TEXT   NOT NULL,          -- tokenURI(); '' is a legitimate value
    discovery_block  BIGINT NOT NULL,          -- the block these reads were pinned to
    discovered_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Set once a census run has swept this id, so the tail can stop showing
    -- it without deleting the record of when it first appeared. NULL means
    -- "still unchecked". Never used to compute a census figure.
    superseded_by_run UUID REFERENCES runs (run_id),
    PRIMARY KEY (chain, agent_id)
);

-- The two reads this table exists to serve: "what is new on this chain" and
-- "is this specific id known", the second being the search/permalink path.
CREATE INDEX idx_tail_unswept ON registration_tail (chain, agent_id)
    WHERE superseded_by_run IS NULL;
CREATE INDEX idx_tail_owner ON registration_tail (chain, owner);

-- Where the poller resumes, per chain. Separate from the indexer's cursor:
-- that one walks logs for minter attribution, this one records the highest
-- agent id the tail has read, which is what the binary search resumes from.
CREATE TABLE registration_tail_cursor (
    chain            TEXT PRIMARY KEY,
    highest_agent_id BIGINT NOT NULL,
    last_block       BIGINT NOT NULL,
    polled_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Said in the catalogue too, so `\d+ registration_tail` carries the rule and
-- not only this file: someone writing a query against a live database is
-- exactly the reader who needs it, and they are not reading migrations.
COMMENT ON TABLE registration_tail IS
    'Agents seen on-chain since the last census sweep. NOT census data: no '
    'run_id, no check results, never a term in a rate, a finding, an archive '
    'or any published figure. A census number is always a pinned run''s.';

COMMENT ON COLUMN registration_tail.superseded_by_run IS
    'The census run that has since swept this id, NULL while still unswept. '
    'Set by the sweeper when a run finishes (and by the tail poller as a '
    'backstop). Records that the tail is done with the row; never used to '
    'compute anything.';

COMMENT ON TABLE registration_tail_cursor IS
    'Where the tail poller resumes per chain: the highest agent id it has '
    'read, and the block it last pinned to. Distinct from the indexer''s '
    'cursor, which walks logs for minter attribution.';
