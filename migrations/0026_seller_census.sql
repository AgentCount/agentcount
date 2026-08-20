-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0026 — the Seller Census (Instrument 02), METHODOLOGY §10.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- The registration census asks whether the agents everyone counts are real.
-- This instrument asks whether the x402 economy everyone cites is real: who is
-- actually selling, and does it hold up. Its method locked on 2026-08-20
-- before any of these tables existed, which is the order this project works
-- in; the rules themselves are `crates/sellers`, pure and testable without a
-- database.
--
-- ## Why its own tables and not `check_results`
--
-- Same argument as `payments` (0019). A seller is not an agent: it has no
-- token id, no owner, no chain of registration, and its identity is a
-- `(payTo, host)` pair rather than a number. Putting seller answers in
-- `check_results` would make them the registration census's rungs by
-- placement, and every rate computed over that table would silently blend two
-- populations that have no member in common.
--
-- ## The run-scoping rule, restated for a population that is not on-chain
--
-- Everything here is scoped to a `seller_run`, exactly as everything in the
-- registration census is scoped to a `run`. But a seller run cannot pin a
-- block: its population comes from catalogs, which are mutable, off-chain, and
-- owned by other people. What replaces the pinned block is
-- `seller_catalog_snapshots` — the hash of the exact bytes each catalog served
-- — so "these were the listings on 2026-W38" stays checkable a year later
-- even if every catalog has since rewritten itself. A run that could not
-- fetch a catalog RECORDS THAT, because a population assembled from four of
-- six catalogs is a different population and must not be silently compared
-- against one assembled from six.
--
-- ## Five tables
--
--   seller_runs                one sweep, its network, its catalog list
--   seller_catalog_snapshots   what each catalog served, hashed
--   seller_population          the deduped (payTo, host) sellers of that run
--   seller_rejected_listings   listings that could not become an identity
--   seller_check_results       one row per (seller, rung)
--
-- The fourth exists because §10.2 requires assembly to be LOSSLESS: a listing
-- this census could not turn into an identity is counted and reported, never
-- dropped. A population assembled by quietly discarding what did not parse is
-- a population nobody can check.

-- ─────────────────────────────────────────────────────────────────────────────
-- seller_runs — one sweep of the Seller Census.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE seller_runs (
    run_id                 UUID        PRIMARY KEY,

    -- Sweep 1 is Base/USDC only (§10.5). Stored per run rather than assumed,
    -- so the stated expansion to another network is a new run with a
    -- different value here and not a reinterpretation of old rows.
    network                TEXT        NOT NULL,

    started_at             TIMESTAMPTZ NOT NULL,
    -- NULL while running. A sweep that died is stamped with the moment it
    -- died and status 'failed' — the same distinction `runs.status` draws,
    -- for the same reason: `finished_at IS NOT NULL` does not mean complete.
    finished_at            TIMESTAMPTZ,
    status                 TEXT        NOT NULL
                                       CHECK (status IN ('running','finished','failed')),

    -- The rules that produced these rows: `sellers::SELLER_CHECKER_VERSION`
    -- and the commit of the code that ran. A stored answer that cannot name
    -- its method is uncitable — the lesson of the 2026-08 census's
    -- `checker_commit: unknown`.
    seller_checker_version TEXT        NOT NULL,
    checker_commit         TEXT        NOT NULL,

    -- THE CATALOG LIST IS PART OF THE METHOD (§10.2). Adding or removing one
    -- changes the population, so it is stored per run: a seller that
    -- disappears because its only catalog was dropped is a method change, not
    -- churn, and the delta must be able to tell those apart by comparing
    -- these arrays rather than by anybody remembering.
    catalogs               TEXT[]      NOT NULL,

    -- Filled when the run finishes. NULL means "not known yet", never zero.
    seller_count           INTEGER,

    rerun_command          TEXT
);

-- ─────────────────────────────────────────────────────────────────────────────
-- seller_catalog_snapshots — what each catalog actually served, hashed.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- This is the pinned block's replacement, and it is the only thing that makes
-- a seller run reproducible: the population is downstream of bytes that other
-- people can rewrite at any time.
--
-- A row exists for every catalog the run ATTEMPTED, including the ones that
-- refused us or fell over. Absence of a row means the catalog was not in this
-- run's list at all — never "we asked and got nothing", which is the same
-- absence-is-not-a-status rule the rest of this schema keeps.
CREATE TABLE seller_catalog_snapshots (
    run_id        UUID        NOT NULL REFERENCES seller_runs (run_id) ON DELETE CASCADE,
    catalog       TEXT        NOT NULL,
    url           TEXT        NOT NULL,
    fetched_at    TIMESTAMPTZ NOT NULL,

    -- 'fetched'  — we have the bytes, and `sha256` names them.
    -- 'refused'  — the catalog declined us (429/503, an auth challenge, or a
    --              robots.txt that said no). Not our failure, not theirs.
    -- 'error'    — OUR failure: a timeout, a TLS error, a parse we could not
    --              complete. Never the catalog's.
    outcome       TEXT        NOT NULL CHECK (outcome IN ('fetched','refused','error')),
    http_status   INTEGER,

    -- NULL unless outcome = 'fetched'. The hash is over the exact response
    -- body, so a later reader can verify the archive they were given is the
    -- one this run read.
    sha256        TEXT,
    byte_len      BIGINT,
    -- How many listings this catalog contributed, before dedup across
    -- catalogs. Per-catalog denominators are a published figure (§10.2's
    -- cross-reference), so they are stored rather than recomputed.
    listing_count INTEGER,

    note          TEXT,

    PRIMARY KEY (run_id, catalog, url)
);

-- ─────────────────────────────────────────────────────────────────────────────
-- seller_population — the deduped sellers of one run.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- The unit is `(payTo, host)` and the primary key says so. Two rules of §10.1
-- are enforced by that key rather than by discipline: the same payTo on two
-- hosts occupies two rows, and the same host quoting two payTos occupies two
-- rows. There is deliberately no "merge" column — groupings (forty sellers
-- sharing one payTo, the aggregator shape) are findings computed over these
-- rows, never edits to them.
CREATE TABLE seller_population (
    run_id    UUID   NOT NULL REFERENCES seller_runs (run_id) ON DELETE CASCADE,

    -- Normalized by `sellers::identity` — EVM lowercased, Solana verbatim.
    -- Normalization happens once, in one place, because a normalization that
    -- differs between the crawler and the delta splits one seller into two and
    -- publishes churn that never happened.
    pay_to    TEXT   NOT NULL,
    -- The FULL host, not the registrable domain: `api.example.com` and
    -- `example.com` are different services. Default ports stripped, IDN in
    -- punycode, non-default port retained as part of the host.
    host      TEXT   NOT NULL,

    -- Which catalogs listed this seller. The union index nobody else has —
    -- "in Bazaar and nowhere else" is a countable claim only because this
    -- column exists.
    catalogs  TEXT[] NOT NULL,
    -- Every priced URL seen for this seller. Sellers and resources are both
    -- published; neither stands in for the other.
    resources TEXT[] NOT NULL,

    PRIMARY KEY (run_id, pay_to, host)
);

-- Per-catalog coverage is `WHERE run_id = $1 AND catalogs @> ARRAY[$2]`, which
-- wants the array indexed rather than scanned.
CREATE INDEX idx_seller_population_catalogs ON seller_population USING GIN (catalogs);

-- ─────────────────────────────────────────────────────────────────────────────
-- seller_rejected_listings — what the population cost to assemble.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- §10.2's losslessness requirement, as a table. Every listing that could not
-- become an identity is here with the reason word (`malformed_address`,
-- `zero_address`, `malformed_url`, `unsupported_scheme`), so "how much of each
-- catalog is unusable, and how" is a question with an answer.
CREATE TABLE seller_rejected_listings (
    id       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_id   UUID   NOT NULL REFERENCES seller_runs (run_id) ON DELETE CASCADE,
    catalog  TEXT   NOT NULL,
    -- As the catalog published them, NOT normalized: the point of this row is
    -- what the catalog said, and normalizing it would erase the evidence.
    pay_to   TEXT   NOT NULL,
    resource TEXT   NOT NULL,
    reason   TEXT   NOT NULL
);
CREATE INDEX idx_seller_rejected_run ON seller_rejected_listings (run_id, catalog);

-- ─────────────────────────────────────────────────────────────────────────────
-- seller_check_results — one row per (seller, rung).
-- ─────────────────────────────────────────────────────────────────────────────
--
-- The seller ladder's answers. Two things the CHECK constraints encode, so
-- that a wrong row cannot be written rather than being caught in review:
--
--   * **Rung 5 is reserved, not absent.** `receipted` is designed (§10.3) and
--     deliberately outside the first locked method until the receipts
--     extension stabilizes. The constraint permits 1,2,3,4,6,7 — a row
--     claiming rung 5 is rejected by the database until the method admits it.
--   * **`unprobed` is a status**, and the sixth word this project uses. It
--     means WE CHOSE NOT TO ASK and `reason` says which of the four §10.4
--     reasons applied. It is never publishable as the seller's failure.
CREATE TABLE seller_check_results (
    run_id     UUID        NOT NULL REFERENCES seller_runs (run_id) ON DELETE CASCADE,
    pay_to     TEXT        NOT NULL,
    host       TEXT        NOT NULL,

    rung       SMALLINT    NOT NULL CHECK (rung IN (1,2,3,4,6,7)),
    name       TEXT        NOT NULL,   -- 'listed' | 'reachable' | 'quotes' | ...
    status     TEXT        NOT NULL
                           CHECK (status IN ('pass','fail','skipped','error','refused','unprobed')),

    -- The machine-readable why, when there is one: an `unprobed` reason
    -- (`over_cap`, `unpriced`, `out_of_scope_network`, `no_quote`) or a
    -- malformed-quote reason. NULL when the status speaks for itself.
    reason     TEXT,

    -- The proof, shaped per rung. Never prose.
    evidence   JSONB       NOT NULL,
    checked_at TIMESTAMPTZ NOT NULL,

    -- The full identity in the key, so a per-seller read SEEKS. The
    -- registration census learned this the expensive way: six queries that
    -- named only the run scanned 1.76 million rows to return 350 (see
    -- `docs/the-chain-trap` and #45). A seller's identity is two columns, and
    -- both of them are here.
    PRIMARY KEY (run_id, pay_to, host, rung)
);

-- Population rates are `GROUP BY rung, status` over one run — the same access
-- path `idx_check_results_rates` serves for the registration census.
CREATE INDEX idx_seller_check_results_rates ON seller_check_results (run_id, rung, status);
