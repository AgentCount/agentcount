# Corrections ledger — the payments round

> **The four corrections below are now enforced in code, 2026-08-06.** Each is
> a named exclusion in `crates/payments/src/exclusions.rs` with a regression
> test citing this ledger by entry, and excluded rows are stored with the rule
> that excluded them so the uncorrected figure stays recomputable. The FIGURES
> here — 313, 190, $8,845,244, $1,090,098 — are **superseded and
> unpublishable**: they came from a study that cannot be recomputed by a sweep.
> See `METHODOLOGY.md` §8 and the 2026-08-06 changelog entry. PAY-4, the fourth
> costume, is recorded in `payments-per-chain.md` §2 rather than here.

Same discipline as FIX 1–8 in `CHANGELOG-METHODOLOGY.md`: what was claimed, why
it was wrong, what it became, and what would have prevented it. **Every
correction here was made before publication.** Three findings were retracted;
none reached a reader.

The pattern across all three is one mistake wearing three costumes: **an address
was treated as an identity.** A shared address was read as many agents, a
wallet's history was read as its later-declared agent's history, and a contract's
balance sheet was read as its controller's earnings.

---

## PAY-1 — payments credited to every agent sharing an address

**Claimed:** 313 agents have received an external payment.

**Cause:** the address→agent mapping is many-to-many, and a payment to a shared
address was credited to *every* agent declaring it. 919 declared plus 347
verified wallets collapse to 846 distinct addresses; one target address is
declared by 62 agents.

**Effect:** 298 addresses received an external transfer, against 313 declaring
agents — 15 agents' worth of double-counting from 5 shared addresses.

**Now:** address-paid and agent-declaring-a-paid-address are reported as separate
numbers, and for the 5 shared addresses the census states plainly that *which*
agent the payment was for is unknowable.

**Would have prevented it:** asking "can two agents name the same address?"
before aggregating. The data answered yes in one query.

---

## PAY-2 — a wallet's history attributed to a later-created identity

**Claimed:** 313 agents paid, $8,845,244 received, median $37.

**Cause:** transfers were counted over the wallet's entire history, but the
wallet existed before the agent did. Nothing checked that a payment arrived
after the agent it was attributed to was minted.

**Effect, measured:** 6,748 of 18,328 external transfers predate the mint of
every agent declaring that address. 222 agents have at least one such transfer,
and for **123 agents every single one** predates their own mint.

| | claimed | corrected |
|---|---:|---:|
| agents with an external transfer | 313 | **190** |
| agents with an x402 settlement | 41 | **36** |
| external transfers | 18,328 | **11,580** |
| x402 settlements | 7,519 | **6,581** |
| external value | $8,845,244 | **$1,090,098** |
| median per agent | $37 | **$16** |

**Value was overstated 8×.** 88% of the money predates the agents it was
credited to.

**Would have prevented it:** treating mint block as the lower bound of an
agent's existence, which is the same rule the census already applies to
`observed_at` everywhere else.

---

## PAY-3 — contract ownership never read, and DeFi flow read as revenue

**Claimed:** one operator earned 97.9% of all agent revenue.

**Cause:** two omissions compounding.

1. **`owner()` of the receiving contract was never read.** The claim assumed the
   NFT owner controlled the money.
2. **The contracts were never inspected**, so vault flows were classified as
   payments.

**Effect, measured:**

* The registrant `0x820c5091…` is an **EOA**. Its 148 agents pay into 148
  per-agent contracts (identical bytecode length, differing code hashes, solc
  0.8.30, not proxies). `owner()` across all 148 returns **126 distinct
  addresses, none of them the registrant.** "One operator earned" was wrong
  about who received the money.
* Those contracts are **not payment contracts**. Their selectors include
  `getAssetProfit`, `allowedVaults`, `getAssetFeesCollected`,
  `assetAvailableVaults`, `adminDeposit`, `getPortfolioSummary`,
  `isAdminApprovedForMerkl`; they emit `Withdrawal(address,address,address,uint256)`.
  The largest senders into them are **Morpho yield vaults** — `Clearstar USDC
  Reactor`, `Steakhouse High Yield USDC v1.1`, `Gauntlet USDC Frontier`. The
  "external payments" are the agent's **own capital returning from DeFi
  positions**, not revenue.

**Now:** external transfers are split by whether the sender has code.

| post-mint external | transfers | value |
|---|---:|---:|
| from a **contract** (vaults, routers, platforms) | 6,127 | **$1,027,924** |
| from an **EOA** (a person or a bot wallet) | 5,453 | **$59,447** |

**94% of the corrected value is contract-sourced**, and for the largest holder it
is provably vault flow. The unambiguous payment signal is the x402 settlement.

**Would have prevented it:** reading `owner()` and the code at the receiving
address — two `eth_call`s. The audit's own instruction ("verify against the
deployed contracts") is exactly what found it.

---

## What survives, and how it may be worded

| claim | status |
|---|---|
| 190 agents received a post-mint external stablecoin transfer | **upper bound**, includes DeFi flow |
| 76 agents received one **from an EOA** | plausibly payments, **$59,447** |
| **36 agents received an x402 (EIP-3009) settlement** | **the only protocol-level evidence of payment** |
| 40,473 agents have `getAgentWallet` set; 347 distinct from the owner | stands |
| 920 documents use the off-chain `agentWallet` convention | stands |
| 409 declared wallets contradict the registry's verified value | stands |
| one registrant holds 148 paid agents | stands — **43** survive the mint correction |
| "one operator earned 97.9%" | **retracted** |
| "313 agents have been paid" | **retracted** |
| "$8.8M received" | **retracted** |

Units, stated once and explicitly: **all values are US dollars.** USDC and USDbC
both use 6 decimals (`decimals()` verified on-chain); every raw log value is
divided by 1,000,000 before reporting. Spot-checked against transaction
`0x459a808c…3d990`, whose raw `Transfer` value of `68289018474` is
**68,289.018474 USDC = $68,289**.

---

## Why this belongs in the launch, not a footnote

The census's whole argument is that a result you cannot recompute is an opinion.
This round is that argument turned on the project itself: the three most quotable
payment numbers it produced were wrong, and its own audit caught all three before
anyone read them.

That is a stronger credibility claim than any finding in the report — and it is
the second time, after FIX 5 retracted this project's own "zero agents caught
writing their own reviews", that the most eye-catching number turned out to be
the least sound.
