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

### A fourth correction, caught in this round: burn addresses are not payees

**Mainnet agent 28283 declares `0x0000000000000000000000000000000000000000` as
its `agentWallet`.**

The declared convention is unverified by construction — nothing in it stops an
agent naming any address at all — so the scan did exactly what it was told and
collected **every USDC and USDT burn on Ethereum mainnet** as an incoming
payment to that agent: **313,255 of mainnet's 314,735 transfers, 99.5%.**

A transfer *to* the zero address is a burn. It is the opposite of revenue.

**Fix:** the zero address and `0x…dead` are excluded from the target set and
from the funnel. Checked across the other chains — **Base and BSC have no such
row**, so no figure already reported changes.

This is the same mistake as PAY-1/2/3 in a fourth costume: **an address was
treated as an identity.** It is also a reminder of what the declared basis is
worth. A `services[]` entry named `agentWallet` is a string in a document
anybody can write; it carries no proof of control, and one of them names an
address that cannot hold anything.

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

---

## 4. Base, re-measured — and it reproduces the corrected findings

Base was re-run through this pipeline from scratch, on the **declared basis
only**, specifically so the BSC comparison is like-for-like. It is also the
pipeline's own validation: Base's corrected numbers were established
independently, by different code, in an earlier session.

| quantity | earlier, 846-address basis | **this run, 825 declared** |
|---|---:|---:|
| agents with an external post-mint transfer | 190 | **181** |
| agents with an x402 settlement | 36 | **32** |
| share of post-mint value from contracts | 94% | **93.7%** |

Both counts land just below the earlier figures, which is the right direction:
this basis is a strict subset of that one. **93.7% against 94% is an
independent reproduction** of the finding that killed the "$8.8M earned" claim.

Base's own scan, in full:

| | value |
|---|---:|
| target addresses scanned | 1,129 |
| incoming stablecoin transfers | 54,788 |
| …post-mint | 48,291 |
| …pre-mint (excluded) | 6,497 |
| **pre-mint share of VALUE** | **82.5%** ($5,508,441 of $6,675,025) |
| external post-mint value | $1,164,083 |
| …from contracts | **93.7%** ($1,092,522) |
| …from EOAs | $74,063 |
| distinct senders | 2,446, of which **1,393 are contracts** |

**The pre-mint correction is the single biggest one on Base: 82.5% of all value
arriving at agent-declared addresses arrived before the agent existed.** By
transfer count it is only 11.9% — a small number of very large early transfers.
Counting them is exactly how the retracted $8.8M was produced.

### x402 is real on Base, and it is micropayments

| | value |
|---|---:|
| x402 settlements (post-mint) | 8,904 |
| addresses receiving one | 44 |
| agents (declared basis) | **32** |
| total value | **$9,489** |
| **mean settlement** | **$1.07** |

Two facts that belong together: Base's stablecoins carried **6,875,861**
`AuthorizationUsed` transactions in the blocks scanned, and **8,904** of them
reached an agent-declared address. **x402 is a busy protocol that ERC-8004
agents are barely part of.**

And where agents do use it, the payments are about a dollar. $9,489 across
8,904 settlements is not a revenue stream; it is a metering mechanism working
as designed. Any framing of x402 volume as "agent earnings" should carry the
mean.

---

## 5. Ethereum mainnet

| | value |
|---|---:|
| agents in the run | 40,806 |
| target addresses scanned | 415 |
| incoming stablecoin transfers | 1,480 |
| …post-mint | 1,235 |
| …pre-mint (excluded) | 245 |
| **agents with an external post-mint transfer** | **31** |
| …of which from an EOA | 22 |
| **agents with an x402 settlement** | **0** |
| external post-mint value | $203,121 |
| share of post-mint value from contracts | 67.8% |

**Mainnet's x402 result is the sharpest version of the Base finding.** Its
stablecoins carried **26,260** `AuthorizationUsed` transactions in the blocks
scanned — the protocol is in use on Ethereum — and **not one of them reached an
agent-declared address.**

So on mainnet and BSC alike, agents are absent from x402 for different reasons:
on BSC the protocol is not used at all, on mainnet it is used and agents are
not part of it.

---

## 6. Celo — the secondary prediction, confirmed harder than expected

Celo has **8,576 target addresses, more than any other chain** — almost every
CeloNova agent declares its own wallet. They are almost all empty.

| | value |
|---|---:|
| agents in the run | 9,747 |
| target addresses scanned | **8,576** |
| incoming stablecoin transfers | 1,001 |
| …post-mint | 853 |
| **agents with an external post-mint transfer** | **18** |
| …of which from an EOA | 15 |
| agents with an x402 settlement | 2 |
| external post-mint value | **$2,813** |

**Celo attests at 79.5% and pays at 0.185% — a ratio of 430 to 1.** The chain
that looks healthiest on every conformance measure has the second-emptiest
payment funnel, and $2,813 of external value across 9,747 agents.

This is the sharpest available demonstration that **attestation and payment are
decoupled**. Celo's attestation rate is three addresses writing feedback for one
platform's batch (`celo.md`); it was never a measure of commerce, and the
payment data shows what was actually underneath it.

Celo's 84 x402 settlements total **$0.94** — a mean of **$0.011**, about a cent.

---

## 7. All four chains

| chain | agents | attested | **paid** | paid rate | from EOA | **x402** | external value | from contracts |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| base | 60,097 | 49.2% | **181** | 0.301% | 70 | **32** | $1,164,083 | 93.9% |
| bsc | 244,208 | 1.8% | **128** | 0.052% | 74 | **0** | $1,639,492 | 86.5% |
| mainnet | 40,806 | 4.1% | **31** | 0.076% | 22 | **0** | $203,121 | 68.1% |
| celo | 9,747 | 79.5% | **18** | 0.185% | 15 | **2** | $2,813 | 50.8% |
| **all four** | **354,858** | **12.2%** | **358** | **0.101%** | 181 | **34** | **$3,009,509** | — |

### What survives

1. **Payment is three orders of magnitude rarer than registration.** 358 agents
   of 354,858 — **1 in 1,000** — have ever received an external post-mint
   stablecoin transfer in scope.
2. **34 agents in the entire four-chain population have ever received an x402
   settlement.** That is **0.0096%**, about 1 in 10,000, and 32 of the 34 are
   on Base.
3. **The thin-BSC prediction holds.** 4.1× Base's agents, fewer paid ones, a
   rate six times lower, and zero x402.
4. **Attestation does not predict payment.** Celo attests 430× more often than
   it is paid; BSC attests at 1.8% and pays at 0.052%. The two measures are
   independent, and no report should let one stand in for the other.
5. **Most value is not revenue.** Contract-sourced flow dominates on every
   chain (50.8%–93.9%), and on Base it is provably Morpho vault yield.

### What does not survive contact with the data

* **"The agent economy."** At 1 in 1,000 paid and 1 in 10,000 settling through
  the ecosystem's own payment protocol, the measured economy is a few hundred
  agents across four chains.
* **Value totals as revenue.** $3,009,509 is the external post-mint inflow to
  agent-declared addresses. It is **not earnings**: most of it is contract flow,
  and the biggest single component was traced to DeFi vaults returning an
  operator's own capital.

### Limits, restated

* **"External" is an upper bound on earnings** — two hops, not a funding graph.
* **Direction, not purpose.** Airdrops, refunds and mistakes look identical to
  payments.
* **Agent counts use the declared map only** (§1), which understates every
  chain equally.
* **Two tokens per chain.** Every figure is a **lower bound**.
