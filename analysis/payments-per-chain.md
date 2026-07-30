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

---

## 1. A method that was tried and rejected — `agentWallet` cannot be read from events

The first attempt built the target address set from **events**, to avoid
354,858 `getAgentWallet` calls: the spec says `agentWallet` "is initially set
to the owner's address", so only *changes* need finding, and changes emit
`MetadataSet(uint256 indexed agentId, string indexed indexedMetadataKey, ...)`
with `topics[2] == keccak("agentWallet")`.

That scan is cheap and complete — 60,480 decoded events on Base alone — and it
produced **19,570 Base agents whose last-set `agentWallet` differs from their
owner**, against the 347 this project had previously measured by calling the
getter. A 56× disagreement.

**The events are right and the inference from them is wrong.** The spec, same
section: *"When the agent is transferred, `agentWallet` is automatically
cleared … and must be re-verified by the new owner."* **Clearing emits no
`MetadataSet`**, so an event-derived "last value" survives a clearing that
actually happened on-chain.

Checked against the live getter at the pinned block, on an evenly-spaced sample
of 99 of those 19,570 agents:

| live `getAgentWallet` at block 49,262,617 | agents |
|---|---:|
| **zero — cleared by a transfer** | **98** |
| equals the event value, and differs from `ownerOf` | 1 |

**~99% of the event-derived set is stale.** Scaled, ~200 of the 19,570 survive,
consistent with the previously measured **347**. The prior figure stands; this
method is rejected.

> **A first sample of 12 said the opposite.** It was the first twelve agents by
> id — ids 0 to 196, all minted in the registry's first hours and never
> transferred — and 11 of 12 matched the live getter. **A sample ordered by the
> variable that drives the effect is not a sample.** The evenly-spaced draw
> reversed the conclusion completely, and this note exists because the biased
> version was believed for several minutes.

**Consequence for what follows.** Basis A (the verified on-chain `agentWallet`)
is used only where confirmed against the live getter. Basis B (the declared
`services[].agentWallet`) is document-derived and exact, and is what the
per-chain funnels below are attributed through. Address *scanning* used the
wider set — scanning extra addresses costs calls, not correctness — but no
transfer is attributed to an agent through a stale mapping.
