# `api` — the public facts API + explorer website

A **binary crate**: the axum web server, and the only crate the outside world
talks to. It reads observations from Postgres, assembles them into
evidence-carrying facts via the pure `facts` crate, and serves them as JSON
and server-rendered HTML. Both surfaces go through the same shared queries —
per-agent facts via `facts_view::assemble`, the paginated directory via
`facts_view::list_agents` — so the site can never disagree with the API.

## Endpoints

**JSON API** — chain is part of every identity path: agent #7 on Base and
agent #7 on Ethereum are different agents.

| Route | Returns |
|-------|---------|
| `GET /api/agents?chain=&limit=&offset=&sort=` | `{items, page:{limit,offset,total}}`. `limit` clamped to 500 (default 100); `sort` is `registered` (default) or `alive` — explicit orderings only, no ranking; an unrecognized `sort` value falls back to `registered`. |
| `GET /api/agents/{chain}/{id}` | Summary + full fact list + flags. Facts and flags each carry a `display` object. |
| `GET /api/agents/{chain}/{id}/facts` | Just the fact list, same published shape. |
| `GET /api/chains` | Enabled chains with agent counts — what a chain filter may offer. |
| `GET /api/stats` | Raw aggregate counts (agents, live, payable, resolving, flagged, flags by kind). |
| `GET /healthz` | Liveness: process up + Postgres reachable. |

**Breaking change (2026-07-27):** `GET /api/agents` returned a bare JSON array
until this date and now returns the `{items, page}` envelope above.

**Server-rendered pages** (askama templates in `../../frontend`)

| Route | Page |
|-------|------|
| `GET /` | Agent directory, newest registration first (a directory, not a leaderboard). |
| `GET /agent/{chain}/{id}` | Facts with evidence + flags with evidence. |
| `GET /methodology` | What we measure and how — including the honest limitations. |
| `GET /static/*` | The stylesheet, served from `frontend/` via `ServeDir`. |

## Files

| File | What's in it |
|------|--------------|
| `src/main.rs` | Router, shared `AppState`, `/healthz`, timeout + concurrency-limit layers. |
| `src/facts_view.rs` | SQL aggregates → `facts::Fact` values, plus the one shared paginated directory query. The ONLY place queries meet the facts crate. |
| `src/error.rs` | `ApiError` + `IntoResponse`/`From` impls (so handlers can `?`). |
| `src/templates.rs` | askama template structs; the display strings they carry are built in `facts::display`, not here. |
| `src/routes/agents.rs` | The JSON agent endpoints. |
| `src/routes/chains.rs` | The chain list for frontend filters. |
| `src/routes/stats.rs` | Aggregate counts. |
| `src/routes/pages.rs` | The HTML page handlers (facts → display rows). |

## Design notes

- **Templates are kept dumb.** All formatting is done in Rust and passed as
  ready-to-print fields — no template-language logic to get wrong.
- **No ranking.** List ordering is explicit and user-visible; a "smart"
  default ranking would be a trust score sneaking back in through the UI.
- **Hardening.** 10s request timeout, 256 in-flight request cap, clamped page
  sizes. Per-IP rate limiting is a fast-follow.
- **Static path.** `ServeDir::new("frontend")` resolves relative to the working
  directory, so run the binary from the workspace root.
- **Every claim is worded once.** Facts and flags each carry a `display`
  object built by `facts::describe` and `facts::describe_flag` respectively:
  facts have `label`, `statement`, and `evidence_summary`; flags have `label`
  and `statement`. The api crate formats nothing itself, so the JSON API, the
  askama pages, and any future frontend state each claim in exactly the same
  words. The raw `value` stays canonical for machine consumers; `display` is
  additive and can be ignored.

## Run it

```sh
export DATABASE_URL=postgres://postgres:dev@localhost:5432/ledgerscope
cargo run -p api            # listens on http://0.0.0.0:8080
```
