# `enricher` — observe endpoints, archive metadata, raise flags

A **binary crate**. The indexer tells us an agent *exists*; the enricher
records what it *does*. It runs periodic passes, each doing three things:

1. **load** agents due for (re-)observation,
2. **observe** each endpoint once — a single SSRF-guarded fetch yields BOTH
   the liveness outcome and an archived metadata snapshot,
3. **flag** coordination patterns across the whole agent set, with evidence.

Everything it learns is **append-only**: probe history, metadata snapshots,
and flag events accumulate; only the small `agent_enrichment` cache (latest
liveness + last good card) is updated in place.

## Files

| File | What's in it |
|------|--------------|
| `src/main.rs` | The pass loop; bounded-concurrency observation via `buffer_unordered`. |
| `src/observe.rs` | One fetch per agent: outcome classification (**HTTP 402 = alive and payable**), capped body reads, sha256 content hashes. |
| `src/netguard.rs` | The SSRF guard: strict URL parsing + DNS resolution + private-range refusal for attacker-controlled domains. |
| `src/flags.rs` | The flag producer: three coordination signals, each flag carrying concrete evidence (peers, addresses, windows). |
| `src/metadata.rs` | The `AgentStub` row shape observation works from. |
| `src/store.rs` | All SQL: loads, append-only observation writes, event-level flag upserts. |

## The flag signals

- **shared_operator** — the same wallet (compared on `address_norm`, so hex
  casing can never fragment a group) owns several agent NFTs.
- **synchronized_registration** — a **burst**: ≥5 registrations with gaps
  under 120s, window capped at one hour. Burst, not chain — naive
  consecutive-gap linking would merge a whole busy afternoon into one
  mega-cluster.

(A third signal, reciprocal feedback, was dropped when the deployed ERC-8004
Reputation Registry turned out to model feedback as client-address→agent rather
than agent→agent. An address-based replacement is future work.)

Flags are per-signal (no cluster merging, no suspicion score, no penalty).
Persistence is append-only at the event level: a new flag inserts a `raised`
event; changed evidence appends `evidence_added`; nothing is ever deleted.

## Concepts it teaches

Bounded concurrency (`buffer_unordered`), failures-as-data (a dead endpoint is
an outcome, not an `Err`), pure-core/async-shell (flag heuristics are plain
functions, unit-tested without a database), and `#[sqlx::test]` for proving
code and schema agree against a real Postgres.

## Run it

```sh
export DATABASE_URL=postgres://postgres:dev@localhost:5432/ledgerscope
export PROBE_CONCURRENCY=32        # optional, default 32
export ENRICH_INTERVAL_SECS=300    # optional, default 300
cargo run -p enricher
```
