# `api` — the public JSON API + explorer website

A **binary crate**: the axum web server, and the only crate the outside world
talks to. It reads everything from Postgres (populated by the indexer and
enricher) and does no heavy computation on the request path — the enricher already
scored every agent — so handlers are simple reads.

## Endpoints

**JSON API**
| Route | Returns |
|-------|---------|
| `GET /api/agents?chain=&limit=` | Leaderboard: agents by latest final score. |
| `GET /api/agents/{id}` | One agent + its latest final score. |
| `GET /api/agents/{id}/score` | The full score breakdown (a `scoring::TrustScore`). |
| `GET /api/stats` | Aggregates, incl. the "how much reputation is fake" fraction. |

**Server-rendered pages** (askama templates in `../../frontend`)
| Route | Page |
|-------|------|
| `GET /` | Explorer leaderboard. |
| `GET /agent/{id}` | Score breakdown with sub-score bars + Sybil warning. |
| `GET /methodology` | The methodology write-up (weights read live from `scoring`). |
| `GET /static/*` | The stylesheet, served from `frontend/` via `ServeDir`. |

## Files

| File | What's in it |
|------|--------------|
| `src/main.rs` | Builds the `Router`, the shared `AppState` (the pool), binds the port. |
| `src/error.rs` | `ApiError` + its `IntoResponse` and `From` impls (so handlers can `?`). |
| `src/templates.rs` | askama template structs; all display strings pre-formatted here. |
| `src/routes/agents.rs` | The three JSON agent endpoints. |
| `src/routes/stats.rs` | The aggregate stats endpoint. |
| `src/routes/pages.rs` | The three HTML page handlers. |

## Concepts it teaches

axum handlers as plain `async fn`s whose *arguments* are extractors (`State`,
`Path`, `Query` — a gentle intro to traits), shared state without globals, errors
that convert into HTTP responses via the `IntoResponse` trait, and compile-time
HTML templating with askama.

## Design notes

- **Templates are kept dumb.** All formatting (percentages, weight strings) is
  done in Rust and passed to the template as ready-to-print fields, so the `.html`
  files only use `{{ }}`, `{% for %}`, `{% if %}` — no template-language logic to
  get wrong, and everything testable in code.
- **Single source of truth for weights.** The methodology page reads
  `scoring::ScoreWeights::default()`, so it can never drift from the real config.
- **Static path.** `ServeDir::new("frontend")` resolves relative to the working
  directory, so run the binary from the workspace root.

## Run it

```sh
export DATABASE_URL=postgres://postgres:dev@localhost:5432/ledgerscope
cargo run -p api            # listens on http://0.0.0.0:8080
```
