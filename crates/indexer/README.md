# `indexer` — chain events → Postgres

A **binary crate**. The mouth of the pipeline: it connects to Ethereum and Base,
watches the three ERC-8004 registry contracts (Identity, Reputation, Validation),
decodes every event they emit, and writes it to Postgres. Everything downstream
(enricher, api) reads what this produces.

## Files

| File | What's in it |
|------|--------------|
| `src/main.rs` | Config from env, opens the pool, spawns one ingest loop **per chain** concurrently. |
| `src/chains.rs` | Connects an alloy provider and bundles it with per-chain registry addresses. |
| `src/bindings.rs` | The `sol!` macro generating typed Rust from ERC-8004 event signatures, plus `index_log` (raw log → decoded + provenance). |
| `src/ingest.rs` | The crash-safe block-follow loop: cursor → fetch logs → decode → persist → advance. |
| `src/store.rs` | All SQL: `load_cursor` and the atomic `write_batch`. |

## How the loop stays correct

- **Resumable cursor.** `indexer_cursor` stores the last fully-processed block per
  chain. A restart resumes from there — no rescanning from genesis, no skipped
  blocks. The cursor is advanced *in the same transaction* as the events it covers,
  so the two can never drift apart.
- **Reorg buffer.** We stay `CONFIRMATIONS` (5) blocks behind the tip so we only
  index blocks unlikely to be reverted by a chain reorganisation.
- **Idempotent inserts.** Every write uses `ON CONFLICT DO NOTHING` keyed on
  `(chain, tx_hash, log_index)` etc., so re-indexing a range is harmless.

## Concepts it teaches

`async`/`.await`, the tokio runtime (`#[tokio::main]`), running concurrent tasks
with `tokio::spawn` + `try_join!`, the `sol!` procedural macro, and `anyhow`-style
error bubbling in a binary.

## ⚠️ Before it will index real data

1. **Registry addresses.** `chains.rs` currently uses `Address::ZERO` placeholders.
   Fill in the real ERC-8004 registry addresses per chain (they differ between
   Ethereum and Base), or it will run happily but find nothing.
2. **Event signatures.** The `sol!` block in `bindings.rs` uses *illustrative*
   signatures. Replace them with the exact ones from the real ABIs (field names,
   types, and `indexed`-ness must match).
3. **A start block.** `DEFAULT_START_BLOCK` in `store.rs` is 0; set it to each
   registry's deployment block so you don't rescan all of history.

## Run it

```sh
export DATABASE_URL=postgres://postgres:dev@localhost:5432/ledgerscope
export ETHEREUM_RPC_URL=https://...   # your provider key
export BASE_RPC_URL=https://...
sqlx migrate run                      # apply the schema first
cargo run -p indexer
```
