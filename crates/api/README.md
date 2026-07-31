# `api` — the public conformance-census API

A **binary crate**: the axum web server, and the only crate the outside world
talks to. It reads runs, agent snapshots, check results, and the HTTP archive
straight from Postgres (written by `crates/sweeper`) and serves them as JSON —
seven rungs per agent, per run, each `pass`/`fail`/`skipped`/`error`, and —
rung 5 (`bound`) only, added 2026-07-29 — `unclaimed`, with evidence. There is
no aggregate field anywhere in the schema and this crate does not invent one.
The Next.js app in the sibling `agentcount-web` repo is the frontend; this
crate serves JSON only.

**Rewrite (2026-07-28):** this crate used to serve a retired availability
model. Its tables (`probe_history`, `metadata_snapshots`, `flags`,
`agent_enrichment`) were dropped in migration 0008, so every endpoint below
had been returning 500s for two days. The whole crate — routes, the pure
library it used to word claims through, everything — was replaced, not
repaired.

## Endpoints

**JSON API** — chain is part of every agent identity path: agent #7 on Base
and agent #7 on Ethereum are different agents. Every endpoint below is scoped
to exactly one run — either an explicit `?run=<uuid>`, or (where noted) the
latest run whose sweep has *finished*, never an in-flight one.

| Route | Returns |
|-------|---------|
| `GET /api/runs?chain=` | Every run, newest first, with full provenance (`schema_version`, `checker_version`, `checker_commit`, `spec_commit`, `rerun_command`, `pinned_block`, `agent_count`). |
| `GET /api/runs/{id}/rates` | The headline output: per rung, the count of each status (`pass`/`fail`/`skipped`/`error`/`unclaimed`), plus `agent_count` as the shared denominator. `GROUP BY rung, status` over one run, backed by `idx_check_results_rates`. |
| `GET /api/agents?run=&chain=&rung=&status=&limit=&offset=` | The directory, one page at a time: `{items, page:{limit,offset,total}}`. `rung`+`status` filter to e.g. "everything failing rung 4"; `limit` clamped to 500 (default 100). `run` defaults to the latest completed run. |
| `GET /api/agents/{chain}/{id}?run=` | One agent: its snapshot, every rung this run asked (in rung order, with full evidence), and the HTTP archive summary (status, content-type, size, sha256, final URL — never the body). `run` defaults to the latest completed run. |
| `GET /api/methodology` | `spec_commit`, `checker_version`, `schema_version`, and the rung-4 required-field list — re-exported from `checks`, never restated. |
| `GET /api/healthz` | Liveness: process up + Postgres reachable. Not `/healthz` — that path is reserved on Cloud Run and never reaches the container. |

## Files

| File | What's in it |
|------|--------------|
| `src/main.rs` | Router, shared `AppState`, `/api/healthz`, timeout + concurrency-limit layers. |
| `src/error.rs` | `ApiError` + `IntoResponse`/`From` impls (so handlers can `?`). `NotFound` → 404, `BadRequest` → 400, `Internal` → 500. |
| `src/routes/runs.rs` | `GET /api/runs`, and `latest_completed` — the shared "fill in a missing `run=`" lookup every other handler calls into. |
| `src/routes/rates.rs` | `GET /api/runs/{id}/rates` — the one permitted aggregate: population counts, never a per-agent one. |
| `src/routes/agents.rs` | The directory and single-agent detail. |
| `src/routes/methodology.rs` | The provenance constants and rung-4 field list, served as data. |

## Design notes

- **No per-agent aggregate, anywhere.** Not a pass count, not "5 of 7", not a
  percentage. The frontend renders seven statuses side by side and never sums
  them; if a query ever looks like `COUNT(*) FILTER (WHERE status='pass')
  GROUP BY agent_id`, that is a score with extra steps and does not belong
  here. Base rates (`/api/runs/{id}/rates`) are the one exception: a
  *population* statistic, computed once, over one run.
- **Absence is not a status.** A rung with no `check_results` row was not
  checked — that's different from any status it might have had. Nothing in
  this crate `COALESCE`s a missing row into a value; a sparse rung list is
  returned sparse.
- **Every query is run-scoped.** `agent_snapshots` and `check_results` are
  both keyed by `run_id`; blending rows from two different runs would compare
  an agent to itself across two different points in time as if they were one.
- **Hardening.** 10s request timeout, 256 in-flight request cap, clamped page
  sizes. Per-IP rate limiting is a fast-follow.

## Run it

```sh
export DATABASE_URL=postgres://postgres:dev@localhost:5432/agentcount
cargo run -p api            # listens on http://0.0.0.0:8080
```
