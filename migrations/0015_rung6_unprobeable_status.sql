-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0015 — rung 6 (`live`) ships, and brings a sixth status:
-- `unprobeable`.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- Rung 6 asks whether anything answers at the endpoints an agent declared.
-- Only `http`/`https` entries can be probed, and across the four-chain census
-- 11.0% of declared "endpoints" are not network endpoints at all — CAIP-10
-- chain addresses, email addresses, empty strings, or no `endpoint` field.
--
-- An agent whose every entry is one of those has nothing for this rung to
-- reach, and none of the five existing statuses is honest for it: `pass` would
-- claim a liveness nobody demonstrated, `fail` would punish an agent for
-- declining to publish a URL the spec never required, `skipped` would falsely
-- imply rung 4 stopped it, and `error` would falsely imply this checker
-- malfunctioned.
--
-- It is deliberately NOT `unclaimed`. That word means the agent made no claim;
-- an agent that published a CAIP-10 address made one, it is simply not one a
-- prober can dial. Collapsing the two would erase a real distinction, exactly
-- as collapsing `unclaimed` into `fail` would have.
--
-- See `crates/checks/src/model.rs` (`CheckStatus::Unprobeable`) and
-- `crates/checks/src/rung6_live.rs` for the check logic, and `METHODOLOGY.md`
-- §2 for the published definition.
--
-- Postgres has no `ALTER CONSTRAINT`, so the old CHECK is dropped and a wider
-- one takes its place. Additive only — every value the old constraint accepted
-- is still accepted, so no existing row is affected and no agent's status
-- moves.
ALTER TABLE check_results DROP CONSTRAINT check_results_status_check;
ALTER TABLE check_results
    ADD CONSTRAINT check_results_status_check
    CHECK (status IN ('pass', 'fail', 'skipped', 'error', 'unclaimed', 'unprobeable'));

-- ─────────────────────────────────────────────────────────────────────────────
-- Where a rung-6 probe's raw observation is archived.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- Separate from `http_archive` rather than a `purpose` column on it, for two
-- reasons that both come back to the unit of the row:
--
--   * `http_archive` is keyed per AGENT — it archives the one fetch of that
--     agent's registration document. Rung 6 probes a URL, and one URL is
--     declared by many agents (four hosts carry 59.2% of every declared
--     endpoint in the census). Keying per agent would re-probe `evoevo.ai`
--     26,273 times, which is the thing the sampling budget exists to prevent.
--   * `http_archive` carries `body`, and migration 0012 projects agent names
--     out of it with a trigger. A rung-6 probe reads no body at all — this
--     rung never inspects what answered — so a row here has nothing for that
--     trigger to project and no business passing through it.
--
-- So: one row per (run, url). The join back to an agent is the declared
-- endpoint string, which rung 6's own evidence already records per entry.
CREATE TABLE IF NOT EXISTS endpoint_probes (
    run_id          uuid        NOT NULL REFERENCES runs(run_id) ON DELETE CASCADE,
    -- The URL as dialled, after normalisation. This is the dedupe key: two
    -- agents declaring the same URL share one row and one request.
    url             text        NOT NULL,
    -- Denormalised from `url` so the per-host budget and the published
    -- host-concentration figures are both computable without re-parsing
    -- 125,705 strings in SQL.
    host            text        NOT NULL,
    -- How many DISTINCT declared endpoints across this run resolved to this
    -- URL. The extrapolation weight, and the reason a sampled host's rate can
    -- be stated over a population without inventing per-agent rows.
    declared_by     integer     NOT NULL DEFAULT 1,
    final_url       text,
    http_status     integer,
    -- `timeout`, `tls`, `robots_disallowed`, `ssrf_blocked: …`. Never a
    -- verdict — `crates/checks/src/rung6_live.rs` is the only place that turns
    -- one of these into a status.
    error           text,
    elapsed_ms      integer,
    probed_at       timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, url)
);

-- The probe pass is resumable, and resuming asks "which of this run's URLs
-- have I not done yet". Without this it is a sequential scan per checkpoint.
CREATE INDEX IF NOT EXISTS endpoint_probes_run_host_idx
    ON endpoint_probes (run_id, host);
