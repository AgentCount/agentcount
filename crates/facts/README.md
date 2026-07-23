# `facts` — measurements with their proof attached

A **library crate** (no `main`), and the successor to the deleted `scoring`
crate. It is the one place a raw measurement becomes a published claim.

It is **pure**: no I/O, no clock reads ("now" is always a parameter). The same
inputs always produce the same fact, so the published methodology is
reproducible — anyone can re-derive our claims from our data.

## The rules

- Every `Fact` carries `evidence` — `EvidenceRef`s a reader can go check
  (a tx hash, an archived snapshot id, a probe window, a registry).
- Values are **raw counts, dates, and statuses** — never normalized scores,
  never weights. "Answered 100 of 120 probes" is the fact; whether that's
  "good" is the consumer's threshold.
- Payment-related facts are tiered (verified settlements vs plausible
  payments) and phrased accordingly — "N distinct addresses sent stablecoins",
  never "N customers". (Payment facts land with the shopper in phase 2.)

## Fact kinds

| Kind | Claim | Evidence |
|------|-------|----------|
| `registered_since` | Registration date per chain | the registration tx |
| `endpoint_liveness` | "answered N of M probes" over a 30-day window | the probe window |
| `payable_endpoint` | endpoint answered HTTP 402 (x402 signal); only exists if observed | the probe window |
| `metadata_status` | resolving / rotted (7+ days) / never_resolved | last good snapshot |
| `attestations` | "N recorded on-chain, M mutual" | registry events |
| `validation_proofs` | present / absent / registry_unavailable (per-chain variance) | registry events |

## Files

| File | What's in it |
|------|--------------|
| `src/lib.rs` | Public surface + the tests that pin the phrasing rules. |
| `src/model.rs` | `Fact`, `EvidenceRef`, and the input structs the api assembles from SQL. |
| `src/derive.rs` | One function per fact kind — measurement in, claim out. |

## Run it

```sh
cargo test -p facts         # each test asserts a phrasing/evidence rule
cargo doc -p facts --open
```
