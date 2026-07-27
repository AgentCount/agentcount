# Ledgerscope

A Rust service that watches the on-chain agent economy and establishes **facts**
about every agent registered under ERC-8004 — through direct observation, with
evidence attached to every claim. It indexes registrations across chains,
observes each agent's endpoint over time (an HTTP 402 counts as alive — that's
the x402 "payable" signal), archives metadata snapshots as they rot, and raises
evidence-backed **flags** for coordination patterns.

There is deliberately **no trust score**: with no observable ground truth to
calibrate weights against, any 0–100 number would be aesthetic. We publish
measurements, not judgments — consumers apply their own thresholds.

See [`METHODOLOGY.md`](METHODOLOGY.md) for what each of the seven rungs
checks, what evidence backs it, and what it explicitly does not mean —
published before any findings exist, so the method can be checked before the
numbers can.

The launch artifact is a research post quantifying *how much on-chain agent
registry activity is manufactured*.

> The code doubles as a Rust-learning vehicle: comments explain the
> Rust-specific *why*, aimed at an experienced engineer who is new to Rust.

## The shape of the thing

Four crates in one Cargo workspace. Postgres is the only shared state — the
binaries never talk to each other directly, so a bug in one stage can never
corrupt another. Observation tables are **append-only**: probes, metadata
snapshots, and flag events are history we never rewrite (the longitudinal
record is the moat).

```
             ┌───────────┐      ┌────────────┐      ┌───────────┐
   chains ──▶│  indexer  │─────▶│  Postgres  │◀────▶│    api    │──▶ JSON
             └───────────┘      └────────────┘      └───────────┘
                                   ▲      │                ▲
                                   │      ▼                │
                                ┌────────────┐         ┌──────────┐
                                │  enricher  │         │  facts   │ (pure lib,
                                └────────────┘         └──────────┘  no I/O)
```

| Crate                | Kind    | Job                                                              |
|----------------------|---------|------------------------------------------------------------------|
| `crates/indexer`     | binary  | Ingest ERC-8004 registry events from every enabled chain.        |
| `crates/enricher`    | binary  | Observe endpoints (SSRF-guarded, one fetch per agent), archive metadata snapshots, raise coordination flags with evidence. |
| `crates/facts`       | library | Pure fact derivation: measurements in, evidence-carrying facts out. |
| `crates/api`         | binary  | axum web server: JSON facts API only (Next.js is the frontend).  |
| `migrations/`        | —       | sqlx SQL migrations (the database schema).                       |
| `scripts/`           | —       | `seed_chains.sql` — the chains (and registry addresses) to index. |

## Getting a database up

```sh
# one-liner local Postgres via Docker (or use a native install)
docker run --name ledgerscope-db -e POSTGRES_PASSWORD=dev \
  -e POSTGRES_DB=ledgerscope -p 5432:5432 -d postgres:16

export DATABASE_URL=postgres://postgres:dev@localhost:5432/ledgerscope

cargo install sqlx-cli          # provides `sqlx migrate`
sqlx migrate run                # applies everything in migrations/
```

## Seeding chains (required before indexing)

Chains are **data**, not code. Edit `scripts/seed_chains.sql` with the real
ERC-8004 registry addresses (CREATE2 → identical across chains) and the Base
deploy block, then:

```sh
psql "$DATABASE_URL" -f scripts/seed_chains.sql
```

The indexer refuses to run a chain whose identity registry is the zero
address, so a forgotten edit fails loudly instead of indexing nothing. Also
verify the `sol!` event signatures in `crates/indexer/src/bindings.rs` against
the deployed registry ABIs — a mismatch decodes silently to nothing.

## Environment variables

| Variable            | Used by            | What it is                                              |
|---------------------|--------------------|---------------------------------------------------------|
| `DATABASE_URL`      | all                | Postgres connection string (see above).                 |
| `RPC_URL_<CHAIN>`   | indexer            | JSON-RPC endpoint per enabled chain row, e.g. `RPC_URL_BASE`. |
| `PROBE_CONCURRENCY` | enricher           | How many endpoints to observe at once (default 32).     |
| `ENRICH_INTERVAL_SECS` | enricher        | Seconds between enrichment passes (default 300).        |
| `RUST_LOG`          | all                | Log verbosity, e.g. `indexer=info,enricher=debug`.      |

Use your own RPC provider keys (Alchemy, Infura, a self-hosted node…). Public
endpoints work for experimenting but rate-limit quickly.

## Handy commands

```sh
cargo check                 # fast type-check of the whole workspace
cargo test                  # unit tests + sqlx tests (needs DATABASE_URL)
cargo run -p indexer        # follow chains (needs seeded chains + RPC URLs)
cargo run -p enricher       # observe agents, raise flags
cargo run -p api            # serve http://localhost:8080
cargo doc --open            # render the teaching comments as browsable docs
```
