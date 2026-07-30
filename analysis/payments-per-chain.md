# Per-chain payment funnels

Extends `payments-design.md` from Base to all four swept chains, with the three
corrections from `payments-corrections-ledger.md` applied **from the start**
rather than retrofitted:

1. **Shared addresses** — the address→agent map is many-to-many. Address-level
   and agent-level counts are reported separately, never blended.
2. **Pre-mint** — a transfer is only attributed to an agent if it arrived
   **after that agent was minted**. A wallet's history before the agent existed
   is not the agent's history.
3. **DeFi flow** — incoming transfers are split by whether the sender has code.
   On Base, 94% of the corrected value came from contracts, and for the largest
   holder it was provably Morpho vault flow, not revenue.

---

## 0. Prediction, recorded before the measurement

> **Written 2026-07-30, before any per-chain payment query was run.** The point
> of writing it down first is that the result becomes a test rather than a
> rationalisation. It is reproduced verbatim below whatever the outcome, and
> the git history shows this section committed before the results section
> existed.

**Predicted: BSC's payment funnel is thin — far thinner than its population
share.**

Reasoning, stated so it can be judged wrong:

- BSC is **69% of the swept population** (244,208 of 354,858) but attests at
  **1.8%** against Base's 49.2%.
- Its entire feedback population is **104 distinct addresses** writing 29,507
  entries — the least diverse of any chain.
- **59.6% of its agents inline their document as a `data:` URI**, so its
  registrations are cheap to produce and require no infrastructure.
- **Zero validation registry activity** (`validation-registry.md`).

So the specific prediction: **BSC will show fewer agents with a post-mint
external stablecoin transfer than Base does, despite having four times as many
agents** — and its x402 settlement count will be at or near zero.

**Secondary prediction:** Celo's funnel will also be thin *relative to its
79.5% attestation rate*, because that rate is three addresses attesting one
platform's batch (`celo.md`), not commerce.

**What would falsify these:** BSC showing a payment funnel proportional to its
population (i.e. ~4× Base's paid-agent count), or Celo's paid-agent share
tracking its attestation share.
