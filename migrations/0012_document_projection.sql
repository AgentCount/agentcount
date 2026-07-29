-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0012 — project name/description out of archived bodies, and index
-- them for search.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- The directory listed "Agent #0", "Agent #1", "Agent #2" because the only
-- identity the API could serve was the token id and the owner address. The
-- name has been in the database the whole time — inside `http_archive.body`,
-- the bytes the document actually served — just never projected out of it.
-- 99.92% of parseable documents in run cfbfcc01 carry one (30,006 / 30,031).
--
-- WHAT THIS TABLE IS NOT. It is a projection for display and search, not a
-- judgement. Nothing here decides whether a document is conformant, bound, or
-- anything else: every rung status and every piece of evidence continues to
-- come from `check_results`, written by `crates/checks`. Copying two strings
-- out of a body adds no opinion to the census, and this migration deliberately
-- stops there — `services_status`, `should_gaps` and the rest already live in
-- rung 4's evidence and are read from there, never recomputed here. Two places
-- deriving the same verdict is exactly the drift this project exists to refuse.
--
-- WHY A TRIGGER. The sweeper writes `http_archive`; this table follows from it.
-- A trigger keeps future runs in sync without editing `crates/sweeper`, which
-- is out of scope for the work this migration belongs to. It also means a
-- backfill and a live sweep populate the table by the same code path, so the
-- two cannot disagree.

CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE TABLE agent_documents (
    run_id      UUID   NOT NULL REFERENCES runs (run_id),
    chain       TEXT   NOT NULL,
    agent_id    BIGINT NOT NULL,
    -- NULL when the document had no usable `name`, never '' — an empty string
    -- is a document that supplied a name and left it blank, which is a
    -- different fact from not supplying one, and the UI renders them
    -- differently (a blank name falls back to "Agent #<id>" the same way a
    -- missing one does, but the distinction stays available in the data).
    name        TEXT,
    description TEXT,
    -- Why no fields were projected, when that is the case: 'no_body' |
    -- 'bad_utf8' | 'bad_json' | 'not_an_object'. NULL means the body parsed
    -- and the fields above are what it contained. This is NOT a rung status
    -- and must never be rendered as one — rung 3 (`parseable`) is the only
    -- thing that answers whether a document parses, and it is unaffected by
    -- anything here.
    doc_error   TEXT,
    PRIMARY KEY (run_id, chain, agent_id)
);

-- ── Extraction ──────────────────────────────────────────────────────────────
-- The UTF-8 decode and the JSON parse are guarded SEPARATELY, in nested
-- exception blocks, so a body that is not valid UTF-8 is distinguishable from
-- one that decodes but is not JSON — and neither aborts the surrounding
-- statement. This is the same guard `analysis/should-gap-signatures.md` used,
-- where it reproduced rung 3's own counts exactly (29,811 pass / 2,046 fail /
-- 11 error) against 60,049 archived bodies, which is why it is trusted here.
--
-- STABLE, not IMMUTABLE: `convert_from` depends on the database encoding, so
-- claiming immutability would be a lie told to the planner. This function is
-- only ever called from a trigger and a backfill, never from an index
-- expression, so STABLE costs nothing.
CREATE OR REPLACE FUNCTION ls_document_fields(
    b bytea,
    OUT doc_name text,
    OUT doc_description text,
    OUT doc_error text
) AS $$
DECLARE
    txt text;
    doc jsonb;
BEGIN
    doc_name := NULL; doc_description := NULL; doc_error := NULL;

    IF b IS NULL THEN
        doc_error := 'no_body';
        RETURN;
    END IF;

    BEGIN
        txt := convert_from(b, 'UTF8');
    EXCEPTION WHEN OTHERS THEN
        doc_error := 'bad_utf8';
        RETURN;
    END;

    -- Strip a leading BOM, for parity with rung 3's own parse.
    IF left(txt, 1) = chr(65279) THEN
        txt := substring(txt from 2);
    END IF;

    BEGIN
        doc := txt::jsonb;
    EXCEPTION WHEN OTHERS THEN
        doc_error := 'bad_json';
        RETURN;
    END;

    -- A bare JSON string, number or array is valid JSON but has no fields to
    -- project. Recorded distinctly rather than folded into 'bad_json': the
    -- body did parse, and saying otherwise here would contradict rung 3.
    IF jsonb_typeof(doc) <> 'object' THEN
        doc_error := 'not_an_object';
        RETURN;
    END IF;

    -- Only a JSON string counts. A `name` that is a number, object or null is
    -- left NULL rather than coerced to text — rendering `{"name":{}}` as the
    -- string "{}" in the directory would be this layer inventing an identity
    -- the document never claimed.
    IF jsonb_typeof(doc -> 'name') = 'string' THEN
        doc_name := nullif(btrim(doc ->> 'name'), '');
    END IF;
    IF jsonb_typeof(doc -> 'description') = 'string' THEN
        doc_description := nullif(btrim(doc ->> 'description'), '');
    END IF;
END;
$$ LANGUAGE plpgsql STABLE;

CREATE OR REPLACE FUNCTION ls_project_agent_document() RETURNS trigger AS $$
DECLARE
    f record;
BEGIN
    SELECT * INTO f FROM ls_document_fields(NEW.body);
    INSERT INTO agent_documents (run_id, chain, agent_id, name, description, doc_error)
    VALUES (NEW.run_id, NEW.chain, NEW.agent_id, f.doc_name, f.doc_description, f.doc_error)
    ON CONFLICT (run_id, chain, agent_id) DO UPDATE
        SET name        = EXCLUDED.name,
            description = EXCLUDED.description,
            doc_error   = EXCLUDED.doc_error;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_http_archive_project_document
    AFTER INSERT OR UPDATE OF body ON http_archive
    FOR EACH ROW EXECUTE FUNCTION ls_project_agent_document();

-- ── Search ──────────────────────────────────────────────────────────────────
-- 'simple', not 'english': these are agent names from a global registry, and
-- stemming them against English rules would mangle more than it helps. The
-- generated column is safe to declare STORED because `to_tsvector` with a
-- literal regconfig IS immutable — unlike `ls_document_fields` above.
ALTER TABLE agent_documents
    ADD COLUMN search tsvector
    GENERATED ALWAYS AS (
        to_tsvector('simple', coalesce(name, '') || ' ' || coalesce(description, ''))
    ) STORED;

CREATE INDEX idx_agent_documents_search ON agent_documents USING GIN (search);

-- Trigram index for fuzzy name matching — 60k rows per run does not need
-- Elasticsearch, and this handles the "I typed it slightly wrong" case that
-- full-text alone does not.
CREATE INDEX idx_agent_documents_name_trgm ON agent_documents USING GIN (name gin_trgm_ops);

-- ── Backfill ────────────────────────────────────────────────────────────────
-- Every run already on disk, through the same extraction the trigger uses.
INSERT INTO agent_documents (run_id, chain, agent_id, name, description, doc_error)
SELECT ha.run_id, ha.chain, ha.agent_id, f.doc_name, f.doc_description, f.doc_error
FROM http_archive ha,
     LATERAL ls_document_fields(ha.body) f
ON CONFLICT (run_id, chain, agent_id) DO NOTHING;
