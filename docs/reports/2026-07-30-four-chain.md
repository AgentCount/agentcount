# ERC-8004 conformance census — four chains, 2026-07-30

354,858 agents. Base, BNB Chain, Ethereum mainnet, Celo, each pinned to a
block.

This report supersedes
[`2026-07-29-base-cfbfcc01.md`](2026-07-29-base-cfbfcc01.md), which covered
Base alone. "Supersedes" means later and wider, not republished: **nothing in
this project has been public.** That report's numbers remain correct for the
chain and run they describe, and §1 below is about what happens to their
*meaning* once three more chains exist.

See [`METHODOLOGY.md`](../../METHODOLOGY.md) for the rules behind every rung and
[`CHANGELOG-METHODOLOGY.md`](../../CHANGELOG-METHODOLOGY.md) for every
correction this project has made to itself, including the ones in this report.

**What this census found, in two facts:**

> Base attests at 49.2%. Across all four chains the rate is **12.2%** — and
> Base, at 16.9% of the population, supplies **68.3% of every attested agent in
> existence.**

> Of those 354,858 agents, **358 have ever been paid** and **34 have ever
> received a settlement through x402, the ecosystem's own payment protocol.**
> One in 991, and one in 10,437. Checked against an independent index that has
> no concept of ERC-8004: **of its 138 largest x402 sellers, none is a
> registered agent's declared wallet.**

---

## 1. The correction this report exists to make

The Base report's headline was 49.2% attested. That number is correct. As a
statement about ERC-8004 it was wrong, and only more chains could show it:

| chain | agents | rung 1 `registered` | rung 2 `resolvable` | rung 7 `attested` |
|---|---:|---:|---:|---:|
| bsc | 244,208 | 100% | 74.0% | **1.8%** |
| base | 60,097 | 100% | 52.8% | **49.2%** |
| mainnet | 40,806 | 100% | 43.2% | **4.1%** |
| celo | 9,747 | 100% | 97.4% | **79.5%** |
| **all four** | **354,858** | **100%** | **67.5%** | **12.2%** |

The spread is 44×. Any single chain, presented as "ERC-8004", is a claim about
that chain's most active platform.

**And the aggregate is barely better than the parts.** 12.2% is the
population-weighted rate, but it is not a fact about a healthy tail: **68.3% of
all attested agents are on Base and 17.9% are on Celo, where — see §6 — three
addresses wrote 99.8% of the attestations for the platform that owns 88% of the
chain.** Both numbers belong in the same sentence, every time.

### The gas hypothesis is dead

The obvious explanation for the spread is transaction cost: attestation is a
write, writes cost gas, so cheap chains should attest more. The data refuses it:

| chain | fee level | attested |
|---|---|---:|
| mainnet | expensive | 4.1% |
| base | cheap | 49.2% |
| bsc | cheap | **1.8%** |
| celo | cheap | 79.5% |

Base and BSC are both cheap L1/L2s and differ by **27×**. Mainnet is the most
expensive chain and outperforms BSC. **Fee cost does not explain attestation**,
and no future report should reach for it. What does explain it, on the two
chains examined closely, is the presence or absence of a single platform
running an attestation service.

## 2. What was measured

Four runs, each pinning a block so that every agent's state is read from one
simultaneous moment rather than assembled over hours:

| chain | run_id | agents | pinned block | registry deployed at |
|---|---|---:|---:|---:|
| bsc | `f78c7891-e787-43f1-9748-61d5a361e9ff` | 244,208 | 112,874,357 | 79,027,268 |
| base | `cfbfcc01-fdaf-409f-9bed-abf706d865c7` | 60,097 | 49,262,617 | 41,663,783 |
| mainnet | `18a25593-9098-40fd-a0d2-75553c6ee31d` | 40,806 | 25,640,407 | 24,339,871 |
| celo | `7833fc49-a5b7-477b-99ce-946f650f0064` | 9,747 | 73,448,013 | 58,396,724 |

The Identity Registry is at `0x8004a169…` on all four, CREATE2-deployed to the
same address; deploy blocks were measured by binary search on `eth_getCode`,
verified on both sides of each boundary.

**Two population footnotes, both from the runs' own manifests:**

* **BSC discovered 244,285 agents and swept 244,208.** 77 could not be read
  from the chain. They are **excluded from the run rather than recorded as
  failures** — an RPC failure is the census's problem, not the agent's. They
  hold no feedback (§4 reconciles this exactly), so they cost nothing but the
  count.
* **Base discovered 60,098 and swept 60,097.** The one missing agent turns out
  to matter a great deal. See §4.

Rung 6 (`live`) is **not implemented**, so it is **absent** from every result
rather than reported as `skipped` or `fail`. "We did not ask" and "we asked and
got no answer" are different claims and the schema keeps them different. §8
says when it ships and why it has not.

## 3. Rung 2 mostly measures who inlines their document

The single most useful thing in the four-chain data is that a rung's *meaning*
changed once there was something to compare against.

**90.9% of Celo's agents put their whole registration document on-chain as a
base64 `data:` URI.** On Base it is 25.8%, mainnet 23.6%, BSC 59.6%. And an
inline document passes rung 2 at **100%**, on every chain, because there is no
network involved:

| chain | inline `data:` | rung 2 pass | fetched over the network | rung 2 pass |
|---|---:|---:|---:|---:|
| base | 15,513 | **100.0%** | 44,584 | 36.3% |
| bsc | 145,665 | **100.0%** | 98,543 | 35.7% |
| mainnet | 9,613 | **99.9%** | 31,193 | 25.7% |
| celo | 8,856 | **100.0%** | 891 | 71.6% |

This is **correct, and the spec asks for it** — line 52 permits any URI scheme
and lines 192–195 recommend a base64 `data:` URI for fully on-chain metadata.
Rung 2 asks whether the registration document can be retrieved; for an inline
document the answer is yes by construction.

So: **rung 2's cross-chain spread is largely a measure of how many registrants
inline, not of how many run a working server.** Celo's 97.4% and BSC's 74.0%
are inlining rates wearing a reachability label. Where a document *is* fetched
over the network, the four chains converge tightly — 25.7% to 36.3% — and
**that** is the reachability finding: roughly two-thirds of agents that point
at a server do not get a document back from it.

This is stated as a limitation of the measure, not a defect in the agents.

## 4. Reputation is machine-written, and Base's feedback is two-thirds unseen

Rung 7 counts feedback entries. Until now the census had never read one: the
Reputation Registry returns a value with every entry and
`chain::Reputation::feedback` kept only the count.

### The values

Reading `NewFeedback` directly (`analysis/feedback-values.md`):

| chain | entries | distinct clients | distinct tags | share of entries at their tag's modal value |
|---|---:|---:|---:|---:|
| base | 427,867 | 12,381 | 570 | **83.3%** |
| bsc | 29,507 | **104** | 34 | 68.7% |
| celo | 27,532 | 4,234 | 239 | 78.1% |
| mainnet | 3,209 | 649 | 185 | 66.0% |

The largest single tag in the entire dataset — `miner-vouch`, **284,066
entries** — has **exactly one distinct value**. Two tags recur across all four
chains with near-identical values: `trust` = 85 (78.8–94.4% of its entries) and
`liveness` = 100 (95.8–99.3%). BSC's **entire** feedback population is 104
addresses. And from rung 7's own evidence, **89.1% of BSC's attested agents
have feedback from exactly one author** (mainnet 88.8%, celo 74.3%, base
53.5%).

**What this does and does not mean.** An automated liveness prober writing
`100` every time a check succeeds is behaving correctly and honestly —
identical values are what a working automated check looks like. The finding is
that **the reputation layer is largely machine-written**, not that it is
dishonest. Nothing here re-derives any value or verifies that a `trust` of 85
reflects trust.

### The one agent Base could not read holds 66.4% of its feedback

The event scan reconciles **exactly** with the census's own stored counts on
BSC, Celo and mainnet — entry for entry. On Base it resolves to the digit:

```
143,713  what rung 7 recorded
+ 284,072  agent 25975
+      82  revoked feedback (getSummary omits it by design)
= 427,867  every NewFeedback event on Base
```

**Agent 25975 holds 284,072 feedback entries from 62 clients — 66.4% of every
feedback entry ever written on Base — and has no row in the run.** It is the
single agent the manifest already records as `unreadable`. The sweeper was
right: a failed chain read is the census's problem, not the agent's, so the
agent is excluded rather than failed, and the count was recorded durably. The
failure was transient — `ownerOf(25975)` answers at the pinned block and now.
**What had never been done was joining that `1` to anything.**

> **No published number becomes wrong. "Feedback on Base" as an unqualified
> phrase becomes unsupportable.** Every Base feedback figure in this report and
> its predecessor covers **33.6% of the chain's feedback events**, and says so
> wherever it appears.

## 5. The Validation Registry — 23 agents out of 354,858

ERC-8004 has three registries. The census reads two. The third had never been
looked at: `chains.validation_registry` is NULL for every chain, which the code
reads as "absent", and the `validations` table has zero rows.

Searching for **the spec's own event topics with no address filter** — because
filtering on NULL returns zero and would have confirmed the assumption —
finds that it is deployed, and used:

| chain | requests | responses | registries | validators | **agents validated** |
|---|---:|---:|---:|---:|---:|
| base | 74 | 68 | 9 | 5 | **19** |
| celo | 31 | 27 | 2 | 22 | **4** |
| bsc | 0 | 0 | 0 | 0 | **0** |
| mainnet | 0 | 0 | 0 | 0 | **0** |
| **total** | **105** | **95** | **10** | **27** | **23** |

**23 of 354,858 agents — 0.0065%.** The mechanism has never been used at all on
BSC or Ethereum mainnet: 285,014 agents, 80% of the population, zero events.

No canonical Validation Registry was ever deployed by the ERC-8004 team — the
curated upstream says it is still under design. The ten that exist are
third-party. Nine of the ten answer `getIdentityRegistry()` with the canonical
Identity Registry this census sweeps, so these validations concern censused
agents; **the tenth reverts on that getter**, which the spec requires at line
347, and its 10 events are reported separately rather than pooled.

A mechanism that has not been finalised has not been rejected — 105 requests is
small, but "unused" and "unwanted" are different claims and only the first is
measured. Full detail: `analysis/validation-registry.md`.

## 6. Celo is one platform, and it moves every chain-level number

Celo is the outlier on every measure: highest attestation (79.5%), highest
resolvability (97.4%). It is **one deployer's batch**.

- **Two EOAs hold 87.9%** of the chain (6,934 and 1,630 agents), in *adjacent*
  contiguous mint ranges — ids 169–1,875, then 1,877–9,045 — with a largest
  single run of **1,503 consecutive agents**. No other owner has a run
  longer than 72.
- Their documents self-identify as **CeloNova**, and that set is exactly those
  two owners.
- One of them emits **6,934 documents with a single top-level key shape**
  (100%). Across rung 4, CeloNova produces one SHOULD-gap signature per 2,830
  agents; every other owner, one per 43.
- **Three EOAs wrote the feedback for 6,825 of CeloNova's 6,836 attested agents
  — 99.8%.**

**Limits, stated plainly.** None of the three clients is either owner, and the
Reputation Registry already forbids owner feedback. This census **never reads
ERC-721 approvals**, so it cannot detect feedback from an approved operator —
which the spec also bans (line 217) — and nothing here alleges it. Everything
measured is equally consistent with a legitimate platform minting agents for
its users, hosting their metadata on-chain, and running its own attestation
service.

> The finding is not about CeloNova's conduct. It is that **Celo's 79.5%
> attestation rate is not an ecosystem-level fact** — it is one platform's
> product decisions measured at chain scale.

Full detail: `analysis/celo.md`.

## 7. Concentration is the census's recurring result

Five independent measurements, five times the same shape:

| # | layer | finding |
|---|---|---|
| 1 | **registration** | one registrant holds 148 paid agents on Base; two addresses hold 87.9% of Celo |
| 2 | **reputation** | 104 addresses write all of BSC's feedback; 3 write 99.8% of CeloNova's; one agent holds 66.4% of Base's |
| 3 | **payments** | 358 paid agents across four chains; 32 of the 34 that ever settled through x402 are on one chain, and none of x402's 138 largest sellers is a registered agent |
| 4 | **infrastructure** | below |
| 5 | **validation** | 27 validator addresses in the entire ecosystem; one agent holds 20% of all requests |

### Infrastructure — measured without probing anything

The rung-6 groundwork data, across all four chains, for agents that reached
rung 4. **141,299 declared service entries**:

| endpoint kind | entries | share |
|---|---:|---:|
| `https` | 125,583 | 88.9% |
| CAIP-10 chain address | 12,811 | 9.1% |
| no `endpoint` field | 1,435 | 1.0% |
| other / not a URL | 947 | 0.7% |
| email address | 351 | 0.2% |
| `http` | 122 | 0.1% |
| empty string | 36 | <0.1% |
| `ipfs` | 14 | <0.1% |

**11.0% of declared "endpoints" are not network endpoints at all** — they are
chain addresses, email addresses, empty strings, or absent.

The 125,705 HTTP(S) endpoints resolve to **3,399 distinct hosts**:

| host | endpoints | share | cumulative |
|---|---:|---:|---:|
| evoevo.ai | 26,273 | 20.9% | 20.9% |
| api.celonova.xyz | 25,455 | 20.2% | 41.2% |
| api.signalbound.art | 11,880 | 9.5% | 50.6% |
| github.com | 10,868 | 8.6% | 59.2% |
| celonova.xyz | 8,490 | 6.8% | 66.0% |
| platform-backend.prod.termix.live | 5,792 | 4.6% | 70.6% |
| q402.quackai.ai | 4,674 | 3.7% | 74.3% |
| nookplot.xyz | 3,246 | 2.6% | 76.9% |

**Four hosts carry 59.2% of every declared endpoint in the census; eight carry
76.9%.** Folding CeloNova's two hosts together makes it the largest single
operator at **27.0%**, and **CeloNova plus evoevo.ai account for 47.9% of all
declared endpoints across four chains.**

An "open-ended agent economy" whose declared service surface is half two
hostnames is a finding about the ecosystem, not about those two operators.

## 8. What this census does not measure

Stated so that absence is never read as evidence.

- **Rung 6 (`live`) — whether any declared endpoint answers.** Not implemented,
  therefore absent from every result, never a failure. It ships as the first
  follow-up report. It has not shipped because the method needs settling (402
  is payment-gated, not dead; a 404 on GET may front a POST-only service;
  liveness ≠ functionality) and because **the probe's User-Agent must first
  carry a domain that resolves and a mailbox that answers** — currently it
  promises neither.
- **Every other chain.** Solana, Arbitrum, Polygon, Optimism and 25 more
  networks carry the same CREATE2 registries. Robinhood Chain (id 4663) has
  both registries deployed and **zero agents minted**. This bounds the
  cross-check in §9 too: 21 of x402scan's top sellers are Solana addresses,
  which our census cannot evaluate in either direction.
- **Batch-settled x402.** Only per-transfer EIP-3009 authorisations are
  visible. Any settlement aggregated off-chain is invisible.
- **Funding graphs beyond two hops.** "External" means the sender is not the
  agent's owner and not any owner in the run. An operator routing funds through
  one fresh intermediary is counted as external, so **"external" is an upper
  bound on genuine earnings**.
- **Off-chain payments, and all non-stablecoin value.** Native ETH, DAI, EURC
  and every other token are out of scope. Every payment figure is a **lower
  bound**.
- **Feedback authenticity.** Whether a value means what its tag says, and
  whether a client is independent of the agent. **ERC-721 approvals are never
  read**, so approved-operator self-feedback is undetectable.
- **Which of seven parties "the agent" is.** Owner, minter, approved operator,
  `agentWallet`, declared wallet, payment-contract controller, agent-controlled
  key and service operator can be eight different parties; the census reads
  three of them. See the role glossary.
- **Intent.** Ordering is not intent. That a wallet earned before its agent was
  registered says the registry counted activity rather than creating it; it
  does not say why.

## 9. Payments

Base's payment findings were retracted three times before publication — 313
agents paid became 190, $8,845,244 became $1,090,098, and "one operator earned
97.9%" was false. All three were one mistake in three costumes: **an address
treated as an identity.** Full record:
[`analysis/payments-corrections-ledger.md`](../../analysis/payments-corrections-ledger.md).

The funnels below apply those corrections **from the start** rather than
retrofitting them, and their predictions were committed **before any
measurement was run** (commit `8996609`) so the result is a test rather than a
rationalisation.

| chain | agents | attested | **paid** | rate | from EOA | **x402** | external value | from contracts |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| base | 60,097 | 49.2% | **181** | 0.301% | 70 | **32** | $1,164,083 | 93.9% |
| bsc | 244,208 | 1.8% | **128** | 0.052% | 74 | **0** | $1,639,492 | 86.5% |
| mainnet | 40,806 | 4.1% | **31** | 0.076% | 22 | **0** | $203,121 | 68.1% |
| celo | 9,747 | 79.5% | **18** | 0.185% | 15 | **2** | $2,813 | 50.8% |
| **all four** | **354,858** | **12.2%** | **358** | **0.101%** | 181 | **34** | **$3,009,509** | — |

"Paid" means **received an external stablecoin transfer after the agent was
minted**, through the agent's own declared wallet. Two stablecoins per chain,
nothing else — every figure is a **lower bound**. Symbols and decimals were read
from each contract: **BSC's USDC and USDT are 18 decimals, not 6**, and Celo's
long-known cUSD address now reports `USDm` at 18.

**Payment is three orders of magnitude rarer than registration.** 358 agents of
354,858 — **1 in 991** — have ever been paid. **34 have ever received an x402
settlement: 1 in 10,437**, and 32 of the 34 are on Base.

**The thin-BSC prediction holds.** BSC has 4.1× Base's agents and fewer paid
ones, at a rate six times lower — on the chain holding 69% of all ERC-8004
agents.

**Attestation does not predict payment.** Celo attests at 79.5% and pays at
0.185% — **a ratio of 430 to 1**. Its attestation rate was three addresses
writing feedback for one platform's batch (§6); it was never a measure of
commerce, and the payment data shows what was underneath it. No report should
let one of these measures stand in for the other.

**x402 is a busy protocol that agents are barely part of.** Base's stablecoins
carried **6,875,861** `AuthorizationUsed` transactions in the blocks scanned;
**8,904** reached an agent-declared address, averaging **$1.07** — metering, not
revenue. Mainnet's carried 26,260 and **not one** reached an agent. BSC has not
a single such event on either token. Agents are absent from x402 on two chains
for opposite reasons: on BSC the protocol is unused; on mainnet it is busy and
agents are not in it.

**Most of the value is not revenue.** Contract-sourced flow dominates
everywhere (50.8%–93.9%), and on Base the largest recipient's inflows are
provably Morpho vault yield — the operator's own capital returning from DeFi.

### Checked against a second source

The 34 is this census's most quotable payment number, so it was checked against
an index built by someone else, by a different method, that has no concept of
ERC-8004: **x402scan** (Merit Systems), which keys settlements by the seller's
receiving address. Full working:
[`analysis/x402scan-crosscheck.md`](../../analysis/x402scan-crosscheck.md).

Our 34 agents resolve to 22 receiving addresses. **Of the 20 on Base, 18
corroborate — 9 of them to the exact settlement count.** The 2 Celo addresses
are outside x402scan's coverage: it indexes base, solana, polygon and optimism,
**not Celo, BNB Chain or Ethereum mainnet.**

Two addresses diverge, and both resolve against their index rather than ours —
but the hypothesis that would have gone the other way was tested first. Our
x402 test is transaction-level, so a batching contract could have inflated us.
It did not: for the divergent address all **1,679 flagged transfers sit in
1,679 distinct transactions**, and `Transfer.from` equals the
`AuthorizationUsed` authorizer in **8 of 8** sampled. The real cause is that
**x402scan's index is facilitator-scoped** — 29 facilitators, 151 relayer
addresses — while ours reads the chain directly; the relayer behind 93% of that
address's settlements appears in none of them. In the opposite case x402scan
reports 1,713 settlements for an address that has only ever received **1,667
USDC transfers in total**, so its unit of count cannot be an incoming transfer.

**No correction to our figures is forced. The 34 stands.**

### And the reverse direction, which is the stronger result

The same index answers a question our data cannot ask of itself: are x402's
*busiest* sellers registered agents?

Taking x402scan's top 100 sellers by volume and top 100 by settlement count —
**138 distinct EVM addresses** — and testing each against our census with a
deliberately generous rule (declared `agentWallet`, **or** the NFT owner, which
is the spec's default value for `agentWallet`, on any of the four chains):

| | |
|---|---:|
| top EVM sellers examined | **138** |
| …that are a declared `agentWallet` | **0** |
| …that are an agent owner | **3** |

And all three sell on Base while owning an agent on a **different chain** —
address reuse by an operator, not an agent being paid.

| | settlements | volume |
|---|---:|---:|
| x402scan's top 100 sellers by volume | 139,636,446 | **$49,617,581** |
| our 20 corroborated agent addresses | 5,717 | **$8,596** |

**Agent-linked value is 0.017% of the top 100's volume**, and our largest agent
seller ($8,436) would not enter a list whose smallest member is $30,648.

Two independent datasets, joined on address, agree: **the payment activity on
x402 is not happening at registered ERC-8004 identities.**

*What this does not establish:* a matching address confirms that a settlement
occurred and who received it — never who initiated it or why. And a
non-matching seller is not "not an agent"; it is not a **registered** ERC-8004
identity on the four chains we sweep. That is the finding: the registry is not
where the payment activity is. x402scan publishes no disclaimer of its own —
its repository contains no methodology page or statement of limits — so this
caveat is ours and is not attributed to them.

**The pre-mint correction is the largest single one.** On Base, **82.5% of all
value arriving at agent-declared addresses arrived before the agent existed** —
only 11.9% of transfers, but the early ones are far larger. Counting them is
exactly how the retracted $8.8M was produced.

### A fourth correction, caught in this round

**Mainnet agent 28283 declares `0x0000…0000` as its `agentWallet`.** The
declared convention is unverified by construction, so the scan collected every
USDC and USDT burn on Ethereum as income to that agent — **313,255 of mainnet's
314,735 transfers, 99.5%**. Burn addresses are now excluded; Base and BSC have
no such row, so no other figure moves.

That is PAY-1/2/3 in a fourth costume, and a fair measure of what the declared
basis is worth: a `services[]` entry named `agentWallet` is a string in a
document anyone can write, and one of them names an address that cannot hold
anything.

### What does not survive contact with the data

* **"The agent economy."** At 1 in 991 paid and 1 in 10,437 settling through the
  ecosystem's own payment protocol, the measured economy is a few hundred agents
  across four chains.
* **Value totals as revenue.** $3,009,509 is external post-mint inflow to
  agent-declared addresses. It is **not earnings**.

### Limits

* **"External" is an upper bound on earnings** — the sender is not the agent's
  owner and not any owner in the run. Two hops, not a funding graph.
* **Direction, not purpose.** Airdrops, refunds and mistakes look identical.
* **Zero means "nothing visible in scope"**, never "never earned anything".
* Agent counts are attributed through the **declared** map only, because the
  verified-`agentWallet` set could not be rebuilt from events
  (`analysis/payments-per-chain.md` §1). This understates every chain equally.

## 10. Reproduction

Every result above traces to one of the four run ids in §2, each of which
stamps `schema_version`, `checker_version`, `checker_commit`, `spec_commit` and
a literal `rerun_command` onto every row it wrote. The pinned spec was
re-verified byte-identical to canonical ERC-8004 on 2026-07-30
(`spec/SOURCE.md`).

Event-derived figures — validation, feedback values, endpoint kinds — are
reproducible with `cast` alone; each analysis document carries its own
commands. Nothing in this report requires access to our database to check.

The x402 cross-check is reproducible without us **or** our method: x402scan's
public tRPC procedures are unauthenticated and free, and
[`analysis/x402scan-crosscheck.md`](../../analysis/x402scan-crosscheck.md)
carries the exact `curl` calls, the upstream commit (`bf7a0cd`), and the query
date. Their documented REST routes are x402-paywalled at $0.01 each; **no
payment was made for anything in this report.**
