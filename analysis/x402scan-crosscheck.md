# Cross-check: our x402 findings against x402scan

> **The 34 is superseded, 2026-08-06.** This cross-check stands as a check of
> the *method* — the sampling test that would have overturned our detection was
> run and failed — but the number it corroborates came from an unpinned study
> and is not quotable. The x402 test it validates (a `Transfer` whose
> transaction carries an `AuthorizationUsed` from the *same* token) is now
> `crates/chain/src/token.rs`, and the pipeline additionally stores the
> authorizer and whether it equals the transfer's sender, so §4a's hypothesis is
> checkable from the rows instead of by sampling. See `METHODOLOGY.md` §8.

Our hardest-won number — **34 agents have ever received an x402 settlement** —
checked against a second, independent index that has no concept of ERC-8004.

x402scan (Merit Systems) indexes x402 settlements keyed by the **seller's
receiving address**. It does not know what an agent is. So the join key is the
payment-receiving address, and the check is genuinely independent: two
different methods, two different data pipelines, one shared set of addresses.

**Headline, and the reason this check exists:**

> Of x402scan's **138 top EVM sellers**, **zero** are a declared ERC-8004
> `agentWallet`. Three are addresses that own an agent — **all three on a
> different chain from the one they sell on.** The busiest sellers on x402 are
> not registered agents.

---

## 1. Provenance

| | |
|---|---|
| source | `github.com/Merit-Systems/x402scan`, commit `bf7a0cde7191f9f2fddaa06d3cd60b5a8f2c21f2` (2026-07-27) |
| queried | 2026-07-30, read-only GETs, identified User-Agent |
| endpoint used | `https://www.x402scan.com/api/trpc/public.sellers.all.list` and `public.facilitators.list` |
| our side | four canonical runs; Base pinned block 49,262,617, Celo 73,448,013 |
| their side | `timeframe: 0` (all time), no upper bound |

**On the API, because the brief asked what access requires:** their documented
REST surface (`/api/x402/merchants`, `/merchants/[address]/stats`,
`/merchants/[address]/transactions`, `/resources`, `/facilitators`) is
**x402-paywalled — every route is declared `.paid('0.01')`**. No payment was
made. The same data is served free and unauthenticated through the public tRPC
procedures their own website calls (`publicProcedure` / `paginatedProcedure` in
`trpc/routers/public/sellers.ts`), which is what this check used. Routes were
read from the repository, not guessed.

**A scope difference that is not a discrepancy:** x402scan's chain enum is
`base`, `solana`, `polygon`, `optimism`. **It does not index Celo, BNB Chain or
Ethereum mainnet.** Our Celo addresses therefore cannot be corroborated there,
and their Solana sellers have no counterpart in our census.

## 2. Our side — 34 agents, 22 receiving addresses

The 34 agents resolve to **22 distinct receiving addresses** (20 Base, 2 Celo);
the fan-out is the shared-address shape PAY-1 warned about — one address is
declared by **10** agents, two others by 3 each.

## 3. Reconciliation, per address

| chain | address | ours (n / $) | x402scan (n / $) | verdict |
|---|---|---:|---:|---|
| base | `0x93862e5b…` | 1,325 / 8,719.83 | 1,297 / 8,435.68 | close |
| base | `0x161dbdd7…` | 1,115 / 81.67 | 1,101 / 80.39 | close |
| base | `0xed32885e…` | 8 / 27.98 | 7 / 23.48 | close |
| base | `0x402c1246…` | **1,679 / 22.07** | **178 / 2.65** | **DIVERGES** |
| base | `0xd34411a7…` | 1,538 / 16.78 | **1,713 / 24.12** | **theirs higher** |
| base | `0x4a82f147…` | 381 / 10.17 | 374 / 9.99 | close |
| base | `0x134820820…` | 3 / 5.99 | 3 / 5.99 | **exact** |
| base | `0x13580b9c…` | 55 / 5.50 | 55 / 5.50 | **exact** |
| base | `0x10800a5a…` | 213 / 1.94 | 192 / 1.71 | close |
| base | `0x5c98e244…` | 132 / 1.81 | 131 / 1.80 | close |
| base | `0x0bbeff1e…` | 55 / 1.22 | 55 / 1.22 | **exact** |
| base | `0x90ee1ebc…` | 204 / 1.10 | 203 / 1.10 | close |
| base | `0xf96c80da…` | 4 / 0.74 | 4 / 0.74 | **exact** |
| base | `0x298a9c08…` | 8 / 0.71 | 8 / 0.71 | **exact** |
| base | `0x75b583c5…` | 349 / 0.35 | 349 / 0.35 | **exact** |
| base | `0xef4dff0f…` | 24 / 0.17 | 23 / 0.17 | close |
| base | `0x6476d358…` | 8 / 0.04 | 8 / 0.04 | **exact** |
| base | `0xdae76a3c…` | 1 / 0.03 | 1 / 0.03 | **exact** |
| base | `0x52ce108a…` | 9 / 0.01 | 9 / 0.01 | **exact** |
| base | `0xf8297111…` | 8 / 222.59 | **absent** | **unconfirmed** |
| celo | `0x0a25c912…` | 73 / 7.30 | 6 / 0.11 *(on **base**)* | not comparable |
| celo | `0xb98cfac3…` | 72 / 0.72 | **absent** | not comparable |

**Of 20 Base addresses: 9 exact, 9 within 15%, 1 materially divergent, 1
absent.** The two Celo addresses are outside x402scan's coverage; one of them
is *separately* active on Base, which x402scan sees and we did not scan for
that address.

## 4. Every discrepancy, and which side is wrong

### 4a. `0x402c1246…` — ours 1,679, theirs 178. **Their gap, not our over-count.**

The obvious suspicion is that our detection over-counts, because our `is_x402`
test is transaction-level: a `Transfer` is flagged if its transaction also
carries an `AuthorizationUsed` from the same token. If a batching contract
moved funds to many recipients under one authorization, we would inflate.

**Tested, and refused.** For this address every one of the 1,679 flagged
transfers sits in its **own transaction** (1,679 transfers, 1,679 distinct
transaction hashes). On an 8-transaction sample, `Transfer.from` equals the
`AuthorizationUsed` authorizer in **8 of 8** cases, with exactly one transfer
per transaction. These are genuine EIP-3009 settlement legs.

**The actual cause: x402scan's index is facilitator-scoped and ours is
chain-scoped.** They index settlements relayed by 29 known facilitators
(151 relayer addresses). We read `AuthorizationUsed` off the chain and are
facilitator-agnostic. Sampling the transaction *senders* behind our 1,679:
one relayer, `0x48380bcf1c09773c9e96901f89a7a6b75e2bbecc`, accounts for **56 of
60 sampled**, and it appears in **none** of x402scan's 151 facilitator
addresses.

### 4b. `0xf8297111…` — 8 settlements, absent entirely. **Same cause.**

Its 8 settlements were relayed by four addresses
(`0xaf2bfb6b…`, `0x048ef106…`, `0x54e2acab…`, `0xa9236f49…`). **None is in
x402scan's facilitator set.** An index built per-facilitator cannot see a
settlement relayed by a party it does not track.

### 4c. `0xd34411a7…` — theirs 1,713, ours 1,538. **We are not missing settlements; the two sides count different units.**

The tempting explanation is that we missed a token — we scan only USDC and
USDbC on Base. So every ERC-20 transfer *ever* received by that address was
pulled, regardless of token:

> **1,667 USDC transfers in total, 2,064 across all tokens.**

**x402scan's 1,713 exceeds the 1,667 USDC transfers that address has ever
received.** Their count therefore cannot be "incoming USDC transfers" — their
unit is something else (plausibly per-authorization or per-facilitator record,
possibly including legs that do not produce a transfer to the seller). Both
sides agree the activity ended around the same time: their
`latest_block_timestamp` is 2026-02-19 and our last flagged settlement for it
is block 43,109,826.

### 4d. Correction forced to our numbers

**None.** Every divergence resolves to their coverage or their unit of count,
and the one hypothesis that would have impugned our figures — transaction-level
over-counting — was tested and failed. **The 34 stands.**

That is worth stating carefully rather than smugly: this check *could* have
overturned our number, and the sampling test was designed so that it would
have. It did not.

## 5. The linkage result

Two rankings pulled from x402scan — top 100 by volume and top 100 by
transaction count — giving **159 distinct sellers, 138 of them EVM addresses**.
Each was tested against our census with a deliberately **generous** membership
rule: does the address appear as a declared `services[].agentWallet`, **or** as
the NFT owner of any agent (the spec's default value for `agentWallet`), on any
of our four chains?

| test | result |
|---|---:|
| top EVM sellers examined | **138** |
| …that are a **declared `agentWallet`** | **0** |
| …that are an **agent owner** | **3** |
| …either | **3 (2.2%)** |

And all three of those are cross-chain coincidences of address reuse — each
sells on Base while owning an agent on a *different* chain:

| seller | sells on | settlements | volume | owns an agent on |
|---|---|---:|---:|---|
| `0x5e50d23e…` | base | 8,904,261 | $2,737,839 | mainnet |
| `0x61d8e97f…` | base | 499 | $146,458 | bsc |
| `0xb5051843…` | base | 284,801 | $6,035 | bsc |

None is "an agent being paid". Each is an operator that separately registered
an ERC-8004 identity somewhere else.

### The scale gap

| | settlements | volume |
|---|---:|---:|
| x402scan's top 100 sellers by volume | 139,636,446 | **$49,617,581** |
| our 20 corroborated agent addresses | 5,717 | **$8,596** |

**Agent-linked value is 0.017% of the top 100's volume.** Our largest agent
seller ($8,436) would not enter the top 100, whose smallest member is $30,648.

## 6. What this does and does not prove

**Proves:**

* The settlements we found are real and independently observed. 18 of 20 Base
  addresses corroborate, 9 of them to the transaction.
* Our method sees settlements a facilitator-scoped index misses — for one
  address, 1,501 of them.
* **x402's busiest sellers are not registered ERC-8004 agents.** Two
  independent datasets, joined on address, agree.

**Does not prove:**

* **Who initiated a payment, or why.** A matching address confirms that a
  settlement occurred and who received it. It says nothing about purpose, about
  whether a service was rendered, or about whether the receiving party is the
  agent as opposed to its operator.
* **That non-matching sellers are not agents in any sense.** They are not
  *registered* ERC-8004 identities on the four chains we sweep. They may be
  agents by any other definition — which is precisely the point: the registry
  is not where the payment activity is.
* **Anything about Celo, BNB Chain or Ethereum mainnet x402 sellers.**
  x402scan does not index those chains.

**A caveat about the caveat.** This section was written expecting to cite
x402scan's own disclaimer about what their data shows. **There is none** — the
repository contains no disclaimer, no methodology page, and no statement of
limits in its README or `docs/DISCOVERY.md`. The limitation above is ours to
state, and is not attributed to them.

## 7. Reproduction

```sh
# routes read from source, not guessed
git clone https://github.com/Merit-Systems/x402scan   # bf7a0cd

# free, unauthenticated, read-only (the REST equivalents are .paid('0.01'))
curl -sG 'https://www.x402scan.com/api/trpc/public.sellers.all.list' \
  --data-urlencode 'input={"json":{"timeframe":0,"sorting":{"id":"total_amount","desc":true},"pagination":{"page":0,"page_size":100}}}'

curl -sG 'https://www.x402scan.com/api/trpc/public.facilitators.list' \
  --data-urlencode 'input={"json":{"timeframe":0,"pagination":{"page":0,"page_size":100}}}'
```
