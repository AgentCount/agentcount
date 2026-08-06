-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0020 — rungs 2 and 6 gain a seventh status: `refused`.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- (Numbered 0020, not 0019: the payments pipeline on `payments/pinned-pipeline`
-- claims 0019, and two migrations sharing a version is a deploy-time failure,
-- not a merge conflict anyone would notice. The gap is harmless — migrations
-- are applied in version order — and closes when that branch lands.)
--
-- The 2026-08 census reported that 19,983 BSC agents had "stopped resolving".
-- 19,962 of them were HTTP 429 — 19,658 from a single host, `metadata.evoevo.ai`
-- — which is traffic this project generated, at a concurrency this project
-- chose. Excluding 429/503, that chain lost 10 agents.
--
-- The checker booked a 429 as rung 2's `fail`, whose published meaning is "their
-- document is unreachable". So an infrastructure problem of ours was recorded as
-- 19,983 separate accusations, and no error rate could see it, because `error`
-- is the word for our failures and `fail` is the word for theirs.
--
-- `refused` is the third thing that actually happened: **the origin is
-- demonstrably there and declined this request.** Five HTTP statuses qualify,
-- in the two groups HTTP itself separates — 429 and 503, the statuses defined to
-- carry `Retry-After`, and 401/402/407, the statuses that answer with a
-- challenge rather than an absence. So do the two `robots.txt` outcomes
-- (`robots_disallowed`, `robots_unavailable: …`), which were `error`: honouring
-- a robots.txt is a decision we made, not a malfunction, and calling it one made
-- mainnet's published error rate 22.1% because 6,133 agents sat behind one host
-- whose `/robots.txt` refused connections.
--
-- It is not `pass` — we did not receive the document — so nothing above it on
-- the ladder moves: `refused` stops a dependent rung exactly as the `fail` or
-- `error` it replaced did. See `crates/checks/src/model.rs`
-- (`CheckStatus::Refused`), `crates/checks/src/refusal.rs` for the one shared
-- predicate, and `METHODOLOGY.md` §2/§4 for the published definitions.
--
-- Postgres has no `ALTER CONSTRAINT`, so the old CHECK is dropped and a wider
-- one takes its place. The constraint change is additive — every value the old
-- one accepted is still accepted — but note that **this is the first status
-- addition that moves existing rows**: `unclaimed` (0011) and `unprobeable`
-- (0015) both named cases nothing had ever been written for. Existing runs are
-- re-judged out of band, by
--
--     DATABASE_URL=… cargo run -p sweeper --bin backfill-refused -- --apply
--
-- and not by this migration, for two reasons. A migration runs unattended on
-- deploy, and rewriting published measurements is not something that should
-- happen because a container restarted. And the reclassification has to re-judge
-- each row through `checks::refusal` rather than a WHERE clause — a second copy
-- of the predicate, in SQL, is a second answer waiting to happen. For audit,
-- the binary's writes are equivalent to:
--
--     UPDATE check_results SET status = 'refused',
--            evidence = jsonb_set(evidence, '{reason}', '"declined"')
--      WHERE rung = 2 AND status = 'fail'
--        AND evidence->>'http_status' IN ('401','407','429','503');
--     UPDATE check_results SET status = 'refused'
--      WHERE rung = 2 AND status = 'fail' AND evidence->>'http_status' = '402';
--     UPDATE check_results SET status = 'refused'
--      WHERE rung = 2 AND status = 'error'
--        AND (evidence->>'reason' LIKE 'robots_disallowed%'
--          OR evidence->>'reason' LIKE 'robots_unavailable%');
--
-- Rung 6's rows are re-judged by re-running the `liveness` pass, which reads the
-- archived `endpoint_probes` and sends no new requests.
ALTER TABLE check_results DROP CONSTRAINT check_results_status_check;
ALTER TABLE check_results
    ADD CONSTRAINT check_results_status_check
    CHECK (status IN ('pass', 'fail', 'skipped', 'error', 'refused', 'unclaimed', 'unprobeable'));

-- ─────────────────────────────────────────────────────────────────────────────
-- The churn series stops being able to report a rate limit as a death.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- `stopped_resolving` is the number nobody else in this ecosystem can produce,
-- and therefore the one that must not lie. A transition into or out of `refused`
-- is now excluded from it — and from `newly_resolving` — by rule, in
-- `crates/sweeper/src/delta.rs`, not by anyone remembering to filter it.
--
-- The exclusion is on these two columns only. `flips` still records every
-- transition, `pass → refused` included: deleting the evidence would make the
-- rate limit invisible, which is the same failure in the other direction. The
-- 19,962 were found by counting them.
COMMENT ON COLUMN run_deltas.stopped_resolving IS
    'Agents whose rung 2 went from pass to a not-pass that is not `refused`. A transition into `refused` (429, 503, an auth/payment challenge, a robots.txt that declined us) is NOT churn: the origin declined us, which is not the agent having gone away. Still counted in `flips`.';

COMMENT ON COLUMN run_deltas.newly_resolving IS
    'Agents whose rung 2 went to pass from a not-pass that is not `refused`. Symmetric with stopped_resolving: getting through this week after being declined last week is not the agent having come back.';
