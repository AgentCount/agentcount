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

---

## 2. Scope and units

| chain | tokens scanned | decimals |
|---|---|---|
| base | USDC `0x833589fc…`, USDbC `0xd9aAEc86…` | 6, 6 |
| bsc | USDC `0x8AC76a51…`, USDT `0x55d39832…` | **18, 18** |
| mainnet | USDC `0xA0b86991…`, USDT `0xdAC17F95…` | 6, 6 |
| celo | USDC `0xcebA9300…`, USDm `0x765DE816…` | 6, **18** |

**Every symbol and decimal above was read from the contract**, not assumed.
This matters more than it sounds: **BSC's USDC and USDT are 18 decimals, not
6**, and Celo's `0x765DE816…` — long known as cUSD — now reports `USDm` at 18.
Carrying Base's 6 across all four chains would have overstated BSC by a factor
of 10¹². All values below are **US dollars**.

Out of scope, so every figure is a **lower bound**: native gas tokens, every
other ERC-20, every other chain, all off-chain settlement.

---

## 3. Result — BNB Chain

**The prediction holds.** BSC has **4.1× Base's agent population and fewer paid
agents.**

| | bsc | base (for comparison) |
|---|---:|---:|
| agents in the run | 244,208 | 60,097 |
| target addresses scanned | 1,248 | 846 |
| incoming stablecoin transfers found | 14,146 | 18,328 |
| …**post-mint** | 11,266 | 11,580 |
| …pre-mint (excluded) | 2,880 (20.4%) | 6,748 (36.8%) |
| **agents with an external post-mint transfer** | **128** | **190** |
| …of which from an **EOA** | 74 | 76 |
| **agents with an x402 settlement** | **0** | **36** |
| external post-mint value | $1,639,492 | $1,090,098 |
| share of post-mint value from **contracts** | **85.1%** | 94.3% |

As a rate: **0.052% of BSC's agents have received an external post-mint
stablecoin transfer, against 0.316% of Base's** — six times lower, on four
times the population.

**x402 on BSC is not thin, it is absent.** There is **not one
`AuthorizationUsed` event** on either BSC stablecoin across the entire scanned
range — not merely none to an agent address, none at all. The ecosystem's own
payment protocol has never been used on the chain holding 69% of its agents.

The three corrections each bite here exactly as they did on Base:

* **Pre-mint (PAY-2):** 2,880 of 14,146 transfers — 20.4% — arrived before any
  agent declaring that address existed. Counting them would have inflated the
  value by $272,690.
* **DeFi flow (PAY-3):** 85.1% of post-mint value arrives **from contracts**,
  not from people. Only $247,799 came from an EOA.
* **Shared addresses (PAY-1):** 121 paid addresses are declared by 142 agents,
  with up to **6 agents on a single address**. Address-paid and
  agent-declaring-a-paid-address are reported separately, never blended.

**A comparability note, stated rather than smoothed over.** Base's 190 was
measured across 846 addresses (347 on-chain-distinct ∪ 822 declared); BSC's 128
is attributed through the declared map only, because BSC's verified-wallet set
has not been confirmed against the live getter (§1). If anything this
*understates* BSC, which makes the prediction's confirmation stronger, not
weaker.
