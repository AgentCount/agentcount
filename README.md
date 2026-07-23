# Ledgerscope

A Rust service that watches the on-chain agent economy and turns raw registry
data into **trust intelligence**. It indexes every agent registered under
ERC-8004 on Ethereum and Base, enriches each with payment history and endpoint
liveness, scores them with a published methodology that discounts manufactured
reputation, and serves it all as a public explorer website plus a free JSON API.

The launch artifact is a research post quantifying *how much of on-chain agent
reputation is fake*.

> ### ⚠️ This repository is a TEACHING SKELETON
>
> It is built to be **read and then filled in by hand**, as a way to learn Rust
> on a real, non-toy architecture. Function bodies are `todo!()`. It is not
> wired to compile green yet — you add dependencies and implementations as you
> go, letting the compiler guide you. Comments explain the Rust-specific *why*,
> aimed at an experienced engineer who is new to Rust.

## The shape of the thing

Four crates in one Cargo workspace. Postgres is the only shared state — the
binaries never talk to each other directly, so a bug in one stage can never
corrupt another.

```
             ┌───────────┐      ┌────────────┐      ┌───────────┐
   chains ──▶│  indexer  │─────▶│  Postgres  │◀────▶│    api    │──▶ web + JSON
             └───────────┘      └────────────┘      └───────────┘
                                   ▲      │                ▲
                                   │      ▼                │
                                ┌────────────┐         ┌──────────┐
                                │  enricher  │         │ scoring  │ (pure lib,
                                └────────────┘         └──────────┘  no I/O)
```

| Crate                | Kind    | Job                                                        |
|----------------------|---------|------------------------------------------------------------|
| `crates/indexer`     | binary  | Ingest ERC-8004 registry events from Ethereum + Base.      |
| `crates/enricher`    | binary  | Probe endpoints, fetch metadata, cluster for Sybil rings.  |
| `crates/scoring`     | library | Pure trust-scoring functions. No I/O, fully unit-testable. |
| `crates/api`         | binary  | axum web server: JSON API + server-rendered explorer.      |
| `migrations/`        | —       | sqlx SQL migrations (the database schema).                 |
| `frontend/`          | —       | askama HTML templates + a little CSS.                      |

## Suggested order to fill it in

Learn from the inside out — start where there's no I/O to distract you:

1. **`scoring`** — pure functions and tests. No async, no network, no database.
   The best place to get comfortable with Rust's types and ownership.
2. **`migrations`** — write the SQL, run it against a local Postgres.
3. **`indexer`** — your first taste of async + alloy. Get one event decoding.
4. **`enricher`** — HTTP with reqwest, then the clustering logic.
5. **`api`** — tie it together and see it in a browser.

## Getting a database up (for when you reach step 2)

```sh
# one-liner local Postgres via Docker
docker run --name ledgerscope-db -e POSTGRES_PASSWORD=dev \
  -e POSTGRES_DB=ledgerscope -p 5432:5432 -d postgres:16

export DATABASE_URL=postgres://postgres:dev@localhost:5432/ledgerscope

cargo install sqlx-cli          # provides `sqlx migrate`
sqlx migrate run                # applies everything in migrations/
```

`DATABASE_URL` matters even at *compile* time: sqlx's checked-query macros talk
to that database during `cargo build` to verify your SQL.

## Environment variables

The binaries read config from the environment at startup. Put these in a `.env`
file (git-ignored) and load it with `set -a; source .env; set +a` before running:

| Variable            | Used by            | What it is                                            |
|---------------------|--------------------|-------------------------------------------------------|
| `DATABASE_URL`      | all + compile time | Postgres connection string (see above).               |
| `ETHEREUM_RPC_URL`  | indexer            | JSON-RPC endpoint for Ethereum mainnet.               |
| `BASE_RPC_URL`      | indexer            | JSON-RPC endpoint for Base.                           |
| `PROBE_CONCURRENCY` | enricher           | How many endpoints to probe at once (default 32).     |
| `RUST_LOG`          | all                | Log verbosity, e.g. `indexer=info,enricher=debug`.    |

Use your own RPC provider keys (Alchemy, Infura, a self-hosted node…). Public
endpoints work for experimenting but rate-limit quickly.

## Handy commands

```sh
cargo check                 # fast type-check of the whole workspace
cargo run -p indexer        # run one binary
cargo test  -p scoring      # test one crate
cargo doc --open            # render these comments as browsable docs
```
