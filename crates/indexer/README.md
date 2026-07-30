# `indexer` — chain events → Postgres

A **binary crate**. The mouth of the pipeline: it connects to every ENABLED
chain in the `chains` table, watches its ERC-8004 registries (on Base: the
Identity Registry's `Registered` event and the Reputation Registry's
`NewFeedback` — Base has no Validation Registry), decodes every event, and
writes it with true on-chain provenance. Everything downstream (enricher, api)
reads what this produces.

Chains are **data, not code**: which chains exist, their registry addresses,
deploy blocks, and reorg buffers all live in the `chains` table (seeded by
`scripts/seed_chains.sql`). Adding a chain is an INSERT plus an
`RPC_URL_<CHAIN>` env var.

## Files

| File | What's in it |
|------|--------------|
| `src/main.rs` | Loads enabled chains from the DB, spawns one ingest loop per chain. |
| `src/chains.rs` | `ChainConfig` (a `chains` row) → connected alloy provider; refuses zero-address registries. |
| `src/bindings.rs` | The `sol!` macro generating typed Rust from ERC-8004 event signatures; `addr_lower` (all addresses stored lowercase); `index_log`. |
| `src/ingest.rs` | The crash-safe block-follow loop: cursor → fetch logs → **fetch block headers** → decode → persist → advance. |
| `src/store.rs` | All SQL: `load_enabled_chains`, `load_cursor`, the atomic `write_batch`. |

## How the loop stays correct

- **Resumable cursor.** `indexer_cursor` stores the last FULLY processed block
  per chain; `resume_from` (unit-tested) restarts at the NEXT block, or at the
  chain's `deploy_block` on first run. The cursor advances *in the same
  transaction* as the events it covers.
- **True block timestamps.** `eth_getLogs` doesn't reliably carry timestamps,
  so the loop fetches the block header for every distinct block that produced
  a log. There is deliberately NO wall-clock fallback — fabricated timestamps
  would poison the longitudinal record permanently.
- **Reorg buffer + detectability.** Each chain's `confirmations` column keeps
  the loop behind the tip, and every raw event stores its `block_hash`, so a
  deeper reorg is detectable and re-processable from the audit log.
- **Idempotent inserts.** Every write uses `ON CONFLICT DO NOTHING` keyed on
  `(chain, tx_hash, log_index)`, so re-indexing a range is harmless.

## ⚠️ Before it will index real data

1. **Seed the chains.** `scripts/seed_chains.sql` carries the real Base ERC-8004
   registry addresses; run it. The `deploy_block` is left at 0 (safe, but scans
   from genesis) — set it to the registry's creation block to skip an hour of
   empty ranges on the first backfill. The indexer refuses zero-address
   registries, so a forgotten edit fails loudly.
2. **Set `RPC_URL_BASE`** (and one var per additional enabled chain).

The `sol!` block in `bindings.rs` already matches the deployed Base registries
(Identity `Registered`, Reputation `NewFeedback`).

## Run it

```sh
export DATABASE_URL=postgres://postgres:dev@localhost:5432/agentcount
export RPC_URL_BASE=https://...       # your provider key
sqlx migrate run                      # apply the schema first
psql "$DATABASE_URL" -f scripts/seed_chains.sql
cargo run -p indexer
```
