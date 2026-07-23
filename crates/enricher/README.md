# `enricher` — add off-chain reality, detect Sybils, score

A **binary crate**. The indexer tells us an agent *exists*; the enricher tells us
whether it's *real*. It runs periodic passes, each doing four things:

1. **load** agents due for (re-)enrichment,
2. **probe + fetch** each endpoint with bounded concurrency,
3. **cluster** the whole agent graph to find Sybil rings,
4. **score** every agent (via the pure `scoring` crate) and persist the results.

## Files

| File | What's in it |
|------|--------------|
| `src/main.rs` | The pass loop; bounded-concurrency probing via `buffer_unordered`. |
| `src/metadata.rs` | Fetch + serde-parse each agent's agent-card JSON. |
| `src/liveness.rs` | Probe an endpoint; classify the result into a `ProbeOutcome` enum. |
| `src/clustering.rs` | The Sybil detector: a **union-find** graph algorithm over shared-operator, synchronised-registration, and reciprocal-feedback edges. |
| `src/scoring_step.rs` | The bridge: assemble a `scoring::AgentView` from DB rows, call `scoring::score`, store the result. |
| `src/store.rs` | All SQL: loads, the enrichment upserts, cluster replacement, score writes. |

## The interesting part: clustering

Sybil detection is a **graph problem**. `clustering.rs`:

1. builds an undirected graph where an edge means "these two agents share a
   tell-tale signal" (same operator wallet, registered within 120s of each other,
   or a mutual A↔B rating);
2. finds connected components with a from-scratch **union-find** (disjoint-set)
   data structure;
3. scores each component's `suspicion` from its internal **edge density** and how
   many *distinct* signals bind it — deliberately **not** size, because the
   scorer's `sybil_penalty` already scales by size (no double-counting).

Each member's suspicion is written back to `agents.suspicion`, where the scorer
reads it.

## Concepts it teaches

Bounded concurrency (`futures::stream::buffer_unordered`), per-item errors that
don't abort the batch (probe failure is *data*, not a crash), modelling outcomes
with an enum instead of a bool, union-find, and calling a pure sibling crate
across a clean I/O boundary.

## Notes

- **Shared-funding-source** is defined as a cluster reason but not yet detected
  (we don't index first-funder data) — see the comment in `clustering.rs`.
- **Feedback scale.** `scoring_step.rs` assumes feedback scores are 0–5
  (`FEEDBACK_SCALE_MAX`); change that constant to match the real ERC-8004 schema.
- **Attester weight** currently uses `1 − suspicion` as a proxy; a fuller system
  would use the attester's own (recursively computed) EigenTrust score.

## Run it

```sh
export DATABASE_URL=postgres://postgres:dev@localhost:5432/ledgerscope
export PROBE_CONCURRENCY=32        # optional, default 32
export ENRICH_INTERVAL_SECS=300    # optional, default 300
cargo run -p enricher
```
