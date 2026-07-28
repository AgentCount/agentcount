-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0010 — the HTTP archive: what each document actually served.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- Off-chain observations cannot be backfilled. A URL that 404s today may have
-- served a perfectly good document yesterday, and no amount of later work
-- recovers it. So we keep the BYTES, not just a verdict: the whole point is
-- that a reader can re-judge our conclusion against the same evidence when the
-- spec, the required-field list, or our own checks change — and they will.

CREATE TABLE http_archive (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_id       UUID   NOT NULL REFERENCES runs (run_id),
    chain        TEXT   NOT NULL,
    agent_id     BIGINT NOT NULL,
    -- What we were asked to fetch, verbatim from tokenURI().
    requested_uri TEXT  NOT NULL,
    -- How we classified it: 'https' | 'http' | 'data' | 'ipfs' | 'empty' | 'unsupported'
    scheme       TEXT   NOT NULL,
    -- The URL actually requested (an ipfs:// URI becomes a gateway URL), and
    -- where we ended up after redirects. NULL when no request was made.
    request_url  TEXT,
    final_url    TEXT,
    http_status  INT,
    content_type TEXT,
    -- Response headers, lower-cased keys. Small and occasionally decisive.
    headers      JSONB,
    -- The body as received, capped. NULL when no bytes were obtained.
    -- BYTEA not TEXT: a body is bytes, may not be UTF-8, and may contain NUL.
    body         BYTEA,
    body_bytes   INT,
    body_sha256  TEXT,
    -- Whether the cap truncated the body. A truncated body must never be
    -- judged as "invalid JSON" — that would blame the agent for our limit.
    truncated    BOOLEAN NOT NULL DEFAULT false,
    -- Our-side failure detail: timeout, dns, tls, robots_denied, gateway_error…
    -- NULL when the fetch completed, whatever the status.
    error        TEXT,
    elapsed_ms   INT,
    fetched_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- No FK to `agents`: that is the retired indexer's table and holds 2,116
    -- rows against a population of 59,999, so it would reject most of the
    -- registry. `agent_snapshots` is the population of record now, and it is
    -- keyed by run_id just like this table.
    CONSTRAINT http_archive_unique UNIQUE (run_id, chain, agent_id)
);
CREATE INDEX idx_http_archive_lookup ON http_archive (run_id, chain, agent_id);
CREATE INDEX idx_http_archive_status ON http_archive (run_id, http_status);
