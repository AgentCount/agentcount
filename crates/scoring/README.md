# `scoring` — the trust methodology, as pure functions

A **library crate** (no `main`). It answers one question: *given everything we
know about an agent, how much should we trust its on-chain reputation?* This is
the intellectual core of Ledgerscope and the part the research post is about.

It is **pure**: data in (`AgentView`), data out (`TrustScore`), zero I/O — no
database, no network, no clock. That's why it can be built and tested completely
on its own, and why its results are deterministic and reproducible (important for
a *published* methodology people are meant to audit).

## The formula

```
raw   = w_pay·payment + w_live·liveness + w_age·age + w_rep·reputation
final = raw · (1 − sybil_penalty)
```

Four positive sub-scores (each in `[0,1]`) are blended by weights; the Sybil
penalty is applied as a **multiplier**. That multiplicative shape is the whole
idea — no amount of good payment/liveness/age/reputation can rescue an agent
that's clearly part of a coordinated farm.

## Files

| File | What's in it |
|------|--------------|
| `src/lib.rs` | Public API (`score`, `score_with_weights`), the combine step, range checks, `ScoringError`. |
| `src/model.rs` | The vocabulary: `AgentView` (input), `TrustScore` (output), `ScoreWeights`, `FeedbackEdge`, `ClusterInfo`. |
| `src/subscores/payment.rs` | Rewards counterparty **diversity** over raw volume (log-scaled). |
| `src/subscores/reputation.rs` | **quality × confidence** — discounts reciprocal rings and low-trust attesters. |
| `src/subscores/liveness.rs` | Fraction of probes that succeeded. |
| `src/subscores/age.rs` | **span × spread** — rewards sustained, not bursty, activity. |
| `src/subscores/sybil.rs` | Turns cluster signals into the `[0,1]` penalty. |

## The anti-gaming ideas (why each sub-score is shaped the way it is)

- **payment** — a self-dealing ring inflates *volume* but not *diversity*, so
  diversity dominates the blend and value is log-scaled.
- **reputation** — a naive weighted average of feedback is 1.0 if every
  sock-puppet says "1.0", so we multiply *quality* (weighted avg) by *confidence*
  (how much credible weight exists). A ring of low-trust, reciprocal raters has
  almost no confidence → almost no reputation.
- **age** — rewarding age alone is gameable (register early, do nothing); we
  multiply span by spread so "500 ratings in one afternoon" scores near zero.
- **sybil** — `suspicion × (1 − 1/clusterSize)`: both a coordination signal and
  cluster size are required to punish.

## Run it

```sh
cargo test -p scoring       # 15 tests, each asserting an anti-gaming property
cargo doc -p scoring --open # browse these comments as rendered docs
```

## Tuning

Every magic number is a named `const` with a comment (`DIVERSITY_K`, `SPAN_K`,
`CONFIDENCE_K`, the default `ScoreWeights`). They're defensible starting points,
**not** tuned against real data — adjust them once you have agents indexed and can
see the score distribution.
