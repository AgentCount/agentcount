# Identity-role audit — run before any payments finding is published

**Verdict: two of the payments findings were wrong, and both would have been
quotable.** The agent-count headline was overstated by 65% and the value
headline by 8×, and the "one operator earned 97.9%" claim was false in the
direction that matters most — it attributed to one party money that reached 126
different ones.

Everything below is verified against the pinned spec and the deployed contracts
at the reference run's block (**49,262,617**, Base). Where the spec and the
deployment disagree, that is reported, not resolved.

---

## 1. Shared-wallet attribution — the headline survives, the wording does not

919 document-declared plus 347 on-chain-verified wallets collapse to 846
distinct addresses, so attribution had to be checked.

**How a payment to a shared address was attributed in the original count:** it
was credited to *every* agent declaring that address. That inflates an
agent-level count whenever an address is shared, and it was never stated.

Agents per target address (845 read; 1 read failure):

| agents declaring the address | addresses |
|---:|---:|
| 1 | 829 |
| 2 | 7 |
| 3 | 4 |
| 4 | 2 |
| 7 | 1 |
| 10 | 1 |
| **62** | **1** |

Restricted to addresses that actually received an external transfer:

| | count |
|---|---:|
| **addresses** with ≥1 external transfer | **298** |
| **agents declaring such an address** | **313** |

So the two numbers must be reported separately, and they are not
interchangeable: 15 of the 313 come from 5 shared addresses (one shared by 10
agents, three by 3, one by 2). For those, **which** of the sharing agents the
payment was for is not knowable — the census can say the address was paid, not
that a particular agent was.

**The 97.9% registrant's 148 paid agents map to 148 distinct receiving
addresses — one per agent.** So that particular count does not suffer from
sharing, and needs no address-level rewording on this ground. It fails for an
entirely different reason (§2).

---

## 2. What the 97.9% address actually is — the finding inverts

`0x820c5091b047b652888f6aa7e1ee615d99f7c8cd` is an **EOA** with no code. Its
agents' payment addresses are **contracts**:

* all sampled receiving addresses carry ~17 KB of code (34,091 hex chars),
  **identical in length**, with **different code hashes** — the same
  implementation deployed per agent with agent-specific immutables, i.e. a
  factory pattern. Compiler metadata says solc 0.8.30. Not a proxy.
* each exposes `owner()`.

Calling `owner()` on **all 148** payment contracts:

| | result |
|---|---:|
| distinct `owner()` values | **126** |
| equal to the agent's NFT owner (`0x820c5091…`) | **0** |
| most-repeated single `owner()` | 6 contracts |

**So "one operator earned 97.9% of all agent revenue" is false.** One registrant
holds 148 agent NFTs; the money those agents received went into 148 separate
contracts controlled by **126 different addresses, none of which is the
registrant**. That is the signature of a **platform registering agents on behalf
of customers**, where each customer controls their own agent's earnings.

What is true, and publishable:

> One registrant (`0x820c5091…`) accounts for 148 of the paid agents. The
> payments went to per-agent contract wallets controlled by 126 distinct
> addresses, none of them the registrant — consistent with a platform
> registering on behalf of customers rather than a single operator earning.

What cannot be claimed either way: whether those 126 addresses are 126
independent people. Nothing on-chain establishes that, and this audit does not
assert it.

**No public attribution was obtained.** No Basescan label or platform
identification was verified, so none is asserted. The architecture is described;
the operator is not named.

---

## 3. Time-varying ownership — and a second, larger problem

Ownership history for all 313 paid agents, from registry `Transfer` events
(0 read failures):

| | agents |
|---:|---:|
| minted and never transferred | 271 |
| **transferred at least once after mint** | **42** |

### Exposure (a): past-owner funding — small

Of the external transfers, **42 transfers across 13 agents** arrived *before the
pinned-block owner acquired the NFT*. Those were classified "external" by
comparing `from` against `ownerOf` at the pinned block, so a previous owner
funding the wallet would have been misread as an outside payment. Material but
minor: 13 of 313 agents, 42 of 18,328 transfers.

### Exposure (b): the payment predates the agent — large

The audit found a bigger problem than the one it was looking for. The payment
contracts existed and were receiving money **before the agents were registered**:

| | count |
|---|---:|
| external transfers arriving **before the agent was minted** | **6,833** of 18,328 |
| paid agents with ≥1 such transfer | **222** of 313 |
| **paid agents where EVERY external transfer predates the mint** | **123** |

For those 123 agents, *none* of the money can be the agent's earnings — the
agent did not exist when it arrived. Counting them as paid is not a
classification edge case, it is attributing a wallet's prior history to an
identity created later.

### Corrected, attribution-safe figures

Restricting to external transfers that arrived **after the agent's own mint**:

| | published | **corrected** |
|---|---:|---:|
| agents with ≥1 external stablecoin transfer | 313 | **190** |
| agents with ≥1 x402 settlement | 41 | **36** |
| external stablecoin value | 8,845,244 | **1,090,098** |
| median per paid agent | 37 | **16** |

**The value figure was overstated 8×.** 88% of the observed money arrived at
those addresses before the agents they are attributed to existed.

190 remains a lower bound on agents paid (scope limits in
`payments-design.md` §5 still apply) and an upper bound on agents that *earned* —
exposure (a) is uncorrected within it, and direction is still not purpose.

---

## 4. Minter vs owner — the batch claim holds

The census does not store the registration transaction sender, so this was
pulled by hand for the six consecutive-id top earners (51455, 51465, 51471,
51483, 51534, 51561). For **all six**:

* mint transaction `from` = `0x820c5091…f7c8cd`
* minted **to** = `0x820c5091…f7c8cd`
* transfers after mint = **0**

So the minter, the first owner and the pinned-block owner are the same address,
the ids are consecutive, and none has moved. **"One batch deployment" is
confirmed** and may stay — provided it is not paired with an earnings claim, per
§2.

**Backlog item, first-class:** capture the registration transaction sender in the
sweeper as a stored field. It is not derivable after the fact without a log
scan, 8004scan displays it separately as CREATOR, and this audit needed it and
had to go outside the census to get it. Until it exists, any report naming a
minter must say it was pulled by hand.

---

## 5. Approved operators — one prose fix

Spec line 217, in full:

> The *agentId* must be a validly registered agent. The *valueDecimals* MUST be
> between 0 and 18. **The feedback submitter MUST NOT be the agent owner or an
> approved operator for *agentId*.** *tag1*, *tag2*, *endpoint*, *feedbackURI*,
> and *feedbackHash* are OPTIONAL.

Audit of surviving prose:

| location | says | verdict |
|---|---|---|
| `METHODOLOGY.md` §rung 7 | quotes line 217 in full, operators included | correct |
| `crates/checks/src/rung7_attested.rs` | quotes it in full | correct |
| frontend `/methodology` rung 7 | said only "the agent's own owner" | **fixed** |

The frontend now quotes the ban in full and states the scope of what is read:
**the census reads `ownerOf` only and never ERC-721 approvals**, so it could not
identify an approved operator even if the contract-level ban did not make the
question moot. No rung, evidence field or report claims otherwise.

---

## 6. Glossary — six roles

Added to the report and to `/methodology` (as `components/RoleGlossary.tsx`, one
source so the two cannot drift). Each entry carries whether the census reads it,
because a role we do not read is a role we must not make claims about.

| role | source | census reads it? |
|---|---|---|
| **NFT owner** | `ownerOf(agentId)` — on-chain, **block-dependent** | yes |
| **Minter** | sender of the registration tx | **no** — not stored (§4) |
| **Approved operator** | ERC-721 `approve` / `setApprovalForAll` | **no** — approvals never read |
| **`agentWallet`** | `getAgentWallet(agentId)` — on-chain, signature-verified | yes |
| **Declared wallet** | a `services[]` entry named `agentWallet` — off-chain, unverified, **not in the spec** | yes |
| **Service operator** | whoever runs the endpoints — in no registry at all | **no** — unknowable |

The sixth role in the request was truncated mid-sentence after "service operator
(off-chain, not in any registry),". **Service operator** is used as the sixth
here. If a different one was intended — payment-contract controller is the
obvious candidate, given §2 made it load-bearing — say so and it will be added;
it is arguably a seventh regardless, since `owner()` of a per-agent payment
contract is neither the NFT owner nor the `agentWallet`.

---

## What this audit changes

* **Do not publish** "313 agents have been paid" or "8.8M received" or "one
  operator earned 97.9%".
* **Publishable**: 190 agents with a post-mint external stablecoin transfer, 36
  via x402, 1,090,098 in value, median 16 — with the address-vs-agent
  distinction (§1) stated and the platform reading (§2) in place of the operator
  one.
* **Unresolved and reported as such**: whether the 126 payment-contract
  controllers are independent parties; why 19,624 agents return a zero
  `getAgentWallet` when the spec says it defaults to the owner; and whether
  exposure (a)'s 13 agents should be reclassified against owner-at-transfer-time
  rather than owner-at-pinned-block.
* **Not yet done**: this audit covers Base only. bsc, mainnet and celo have not
  been examined, and celo already differs from Base on every other measure.
