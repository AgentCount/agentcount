# `api` — the public facts API

A **binary crate**: the axum web server, and the only crate the outside world
talks to. It reads observations from Postgres, assembles them into
evidence-carrying facts via the pure `facts` crate, and serves them as JSON.
The Next.js app in the sibling `ledgerscope-web` repo is the frontend; this
crate serves JSON only.

## Endpoints

**JSON API** — chain is part of every identity path: agent #7 on Base and
agent #7 on Ethereum are different agents.

| Route | Returns |
|-------|---------|
| `GET /api/agents?chain=&limit=&offset=&sort=` | `{items, page:{limit,offset,total}}`. `limit` clamped to 500 (default 100); `sort` is `registered` (default) or `alive` — explicit orderings only, no ranking; an unrecognized `sort` value falls back to `registered`. |
| `GET /api/agents/{chain}/{id}` | Summary + full fact list + flags. The summary, each fact, and each flag all carry a `display` object. |
| `GET /api/agents/{chain}/{id}/facts` | Just the fact list, same published shape. |
| `GET /api/chains` | Enabled chains with agent counts — what a chain filter may offer. |
| `GET /api/methodology` | The measurement windows (`liveness_window_days`, `rot_after_days`) so a consumer states them without hardcoding. |
| `GET /api/stats` | Raw aggregate counts. `flags_by_kind` is an array of `{kind, label, count}`, most-flagged first. |
| `GET /healthz` | Liveness: process up + Postgres reachable. |

**Breaking changes (2026-07-27):** `GET /api/agents` returned a bare JSON array
until this date and now returns the `{items, page}` envelope above.
`GET /api/stats` returned `flags_by_kind` as an object keyed by kind
(`{"shared_operator": 12}`) and now returns an ordered array of
`{kind, label, count}` — an array because a chart needs a stable order, and
with the label attached so no consumer re-derives it.

## Files

| File | What's in it |
|------|--------------|
| `src/main.rs` | Router, shared `AppState`, `/healthz`, timeout + concurrency-limit layers. |
| `src/facts_view.rs` | SQL aggregates → `facts::Fact` values, plus the one shared paginated directory query. The ONLY place queries meet the facts crate. |
| `src/error.rs` | `ApiError` + `IntoResponse`/`From` impls (so handlers can `?`). |
| `src/routes/agents.rs` | The JSON agent endpoints. |
| `src/routes/chains.rs` | The chain list for frontend filters. |
| `src/routes/methodology.rs` | The measurement windows, served as data. |
| `src/routes/stats.rs` | Aggregate counts. |

## Design notes

- **No ranking.** List ordering is explicit and user-visible; a "smart"
  default ranking would be a trust score sneaking back in through the UI.
- **Hardening.** 10s request timeout, 256 in-flight request cap, clamped page
  sizes. Per-IP rate limiting is a fast-follow.
- **Every claim is worded once.** Facts and flags each carry a `display`
  object built by `facts::describe` and `facts::describe_flag` respectively:
  facts have `label`, `statement`, and `evidence_summary`; flags have `label`
  and `statement`. An agent summary carries one too (`status`, `statement`)
  for its endpoint, and `/api/stats` ships each flag kind's `label` beside its
  count — so no consumer ever has to turn a bool or a `snake_case` kind into
  words itself. The api crate formats nothing, so the JSON API and the
  Next.js frontend state each claim in exactly the same words. The raw
  `value` stays canonical for machine consumers; `display` is additive and
  can be ignored.
- **Thresholds are data, not prose.** `facts::LIVENESS_WINDOW_DAYS` and
  `facts::ROT_AFTER_DAYS` are the single definition of the measurement
  windows: the queries use them, `/api/methodology` publishes them, and the
  methodology page interpolates them rather than restating the numbers.

## Run it

```sh
export DATABASE_URL=postgres://postgres:dev@localhost:5432/ledgerscope
cargo run -p api            # listens on http://0.0.0.0:8080
```
