# Payments: what the spec actually says, and how to measure "has ever been paid"

**Analysis and design. No check logic, crate code, or schema was touched.**
Every count below comes from archived bodies and stored `check_results` for the
reference run `cfbfcc01-fdaf-409f-9bed-abf706d865c7` (Base, pinned block
**49,262,617**, 60,097 agents), or from read-only `eth_call`s at that same
block. Nothing here is published yet; it folds into the launch report as one
funnel.

---

## 1. `agentWallet` is in the pinned spec — but not where we were measuring it

The hypothesis under test was that `agentWallet` is an 8004scan profile
extension rather than spec, like `updatedAt` in FIX 3. **It is in the spec, and
that makes the finding sharper rather than weaker — but the thing we measured
is still not the spec's mechanism.**

`spec/ERC8004SPEC.md` line 141:

> The key `agentWallet` is reserved and cannot be set via `setMetadata()` or
> during `register()` … It represents the address where the agent receives
> payments and is initially set to the owner's address. To change it, the agent
> owner must prove control of the new wallet by providing a valid EIP-712
> signature for EOAs or ERC-1271 for smart contract wallets

with `setAgentWallet(agentId, newWallet, deadline, signature)`,
`getAgentWallet(agentId) view returns (address)`, `unsetAgentWallet(agentId)`,
and (line 154) automatic clearing on transfer.

So the spec's payment address is:

* **on-chain**, in reserved registry metadata — not a field of the off-chain
  registration document;
* **cryptographically verified** — changing it requires a signature proving
  control of the new address;
* **self-invalidating** — cleared when the NFT is transferred, so a stale
  address cannot survive a change of owner.

`getAgentWallet` is live on the deployed registry (selector `0x00339509`,
confirmed against Base at the pinned block).

**What we measured instead** was a `services[]` entry whose `name` is
`agentWallet`, inside the off-chain JSON document. That pattern appears
**nowhere** in the pinned spec. `services[]` entries are specified as
`{name, endpoint, version, …}` service descriptors; nothing reserves the name
`agentWallet` or gives it payment semantics. It is a community/8004scan-style
convention.

### Consequences for the framing

1. **The 1.5% figure measures adoption of a convention, not spec conformance.**
   It must be worded that way. It is not a conformance rung and must never be
   rendered as one.
2. **It may undercount payable agents**, exactly as suspected — an agent can be
   payable via the on-chain `getAgentWallet` while its document says nothing, or
   via a service endpoint that answers HTTP 402, or by declaring
   `x402Support: true` with no address field at all. Section 2 quantifies the
   last of these.
3. **It may also *overcount*** reachable-for-payment agents, which the original
   framing missed: a document can name an address that the registry has never
   verified, or that disagrees with the one it has. Section 4 quantifies this.

### One divergence worth flagging, not resolving here

The spec says `agentWallet` "is initially set to the owner's address", but
`getAgentWallet` returns the zero address for some agents (0 and 888, spot
checked). That is consistent with clearing on transfer, with registration paths
that never set it, or with the deployment differing from the spec text — this
document does not claim to know which. The population scan in §5 reports the
distribution; attributing the cause needs the `MetadataSet` /
`Transfer` event history, which the census does not index today.

---

## 2. `x402Support` — the second, independent denominator

Of the 30,253 documents that parse as a JSON object:

| `x402Support` | documents |
|---|---:|
| `true` | **2,080** |
| `false` | 6,599 |
| present but not a boolean | 6 |
| absent | 21,568 |

`x402Support` is a **MAY** field (`spec/REQUIRED_FIELDS.md` Ruling 3 / FIX 2) —
it appears only inside the spec's illustrative example JSON with no normative
sentence, and treating its absence as a defect was reversed. So 21,568 absences
are not failures of anything.

### The gap is the finding

Cross-tabulating "declares `x402Support: true`" against "publishes an
`agentWallet` service entry":

| `x402Support` | publishes wallet entry | agents |
|---|---|---:|
| `true` | yes | **103** |
| `true` | no | **1,977** |
| `false` | **yes** | **695** |
| `false` | no | 5,910 |
| absent | yes | 122 |
| absent | no | 21,446 |

Two claims that ought to travel together barely overlap:

* **1,977 agents declare support for a payment protocol and publish no address
  to pay.** 95% of the `x402Support: true` population.
* **695 agents publish a payment address while declaring `x402Support: false`.**

Union of "any payment signal in the document" = 2,080 + 920 − 103 = **2,897** of
30,253 parsed documents (9.6%), against 920 (3.0%) on the wallet-entry measure
alone. Which denominator is "payable" depends entirely on which signal you
believe, and the two disagree about 2,672 agents.

---

## 3. Reconciling 30,253

The 30,253 figure is neither the report's 30,031 nor 30,284:

| quantity | count |
|---|---:|
| archived bodies that parse as JSON | 30,253 |
| …of which are JSON **objects** | 30,253 (all of them) |
| rung 3 (`parseable`) `pass` | 30,031 |
| **difference** | **222** |

All 222 have rung 3 = **`skipped`**, and all 222 are valid JSON objects.
`skipped` at rung 3 means rung 2 (`resolvable`) did not pass, so rung 3 never
judged them. Migration 0010 archives the response body even when resolution
fails, so an archived 402 or 404 page whose body happens to be valid JSON lands
here. The reverse check returns **zero rows** — no rung-3 pass is missed by the
body-parsing query.

**30,253 = 30,031 + 222.** Truncation is not involved: only **11** bodies in the
run are truncated. The 253 in the 30,284 hypothesis is rung 4's *fail* count,
not a truncation count.

The launch funnel therefore uses **30,031** for "documents that parse", because
that is the rung the census actually publishes, and notes the 222 separately.

---

## 4. The declared address is an unverifiable claim — and 409 times it is contradicted

Of 919 agents whose document declares a wallet entry with a parseable address
(920 declare an entry; one carries no `0x…` address at all), compared against
the spec's own on-chain `getAgentWallet` at the pinned block:

| on-chain `getAgentWallet` vs the document | agents |
|---|---:|
| **matches** the document | **460** |
| **differs** from the document | **409** |
| not set (zero) while the document declares one | **50** |
| read failed after retries | 0 |

And against the NFT owner: **737** documents name an address that is not the
owner, 182 name the owner.

### How to frame this

Not as an accusation. Every one of the 409 has innocent explanations: an
operator wallet legitimately separate from the owner, an on-chain rotation the
document has not caught up with, or a document written before
`setAgentWallet` was used. **The point is that a reader cannot tell.**

What makes it reportable is the asymmetry: the spec provides a mechanism whose
whole purpose is to make this checkable — a signature proving control, cleared
on transfer — and the convention in use bypasses it. A payer who reads the
document gets an address with:

* no proof that the agent controls it,
* no link to the on-chain identity,
* served over mutable HTTP that can change between reads, and
* in 409 cases, disagreeing with the address the registry has verified.

So: **409 agents publish a payment address that the registry contradicts, and
50 publish one the registry has never verified at all.** Whether that is correct
operator separation or a substituted address is not something the standard lets
a payer distinguish — and that is the finding.

### Multiple wallets, and which one a payer would use

| wallet entries declared | agents |
|---|---:|
| 1 | 913 |
| 2 | 3 |
| 3 | 3 |
| 4 | 1 |

920 agents, **932 entries**. Of the 7 agents declaring more than one, **5 repeat
the same address** (harmless duplication) and **2 declare genuinely different
addresses**.

**There is no precedence rule, because the spec does not define this field at
all.** A payer has nothing to consult: no "first wins", no `primary` flag,
nothing. Any implementation that picks the first entry and any implementation
that picks the last will disagree about those 2 agents, and both will be
"correct". This analysis uses the **first** entry by array order and says so;
that choice is arbitrary and is disclosed rather than defended.

---

## 5. Design: measuring "has ever been paid"

### Why balance queries are out

A balance is a scalar at one instant and cannot answer the question:

* **zero balance ≠ never paid** — funds received and swept out leave nothing;
* **non-zero balance ≠ earned** — the owner can fund the address, and for the
  182 agents whose declared wallet *is* the owner, "balance" is just the owner's
  money sitting in the owner's own account.

The measurement must be over **incoming transfers**, classified by counterparty.

### Which addresses

All three, reported separately — never blended into one headline:

| basis | what it means | population |
|---|---|---|
| **A. on-chain `getAgentWallet`** | the spec's canonical, signature-verified address | §6, being counted |
| **B. document `services[].agentWallet`** | the unverified convention | 919 with a parseable address |
| **C. NFT owner** | the contract's own fallback; fleet-level, not per-agent | 60,097 |

Basis **A is primary** — it is the only one the spec endorses and the only one
with a proof of control behind it. **C is reported but must be labelled as
measuring operators, not agents**: one owner holds 2,293 agents, so a payment to
that address says nothing about which agent earned it.

### Which tokens, which chain

**In scope for v1: Base only, two tokens.**

| token | address | why |
|---|---|---|
| USDC (native) | `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` | what x402 settles in on Base |
| USDbC (bridged) | `0xd9aAEc86B65D86f6A7B5B1b0c42FFA531710b6CA` | still circulating; excluding it would undercount |

**Explicitly out of scope, and therefore an undercount:** native ETH, DAI, EURC,
every other ERC-20, every other chain, and every off-chain payment. The result
is a **lower bound**, and must be published as one. It is stated as "received at
least one incoming stablecoin transfer in scope", never as "earned".

### Method

For each target address, incoming ERC-20 `Transfer` logs up to the pinned block —
`eth_getLogs` with `address` = token, `topics[0]` = the `Transfer` signature,
`topics[2]` = the padded recipient. Then classify each transfer by its `from`:

| class | rule | counts as paid? |
|---|---|---|
| owner funding | `from` == this agent's NFT owner | **no** |
| fleet-internal | `from` is the owner of any agent in this run | **no**, reported separately |
| external | anything else | **yes** |

And independently, per transfer, whether it was an **x402 settlement**: EIP-3009
`transferWithAuthorization` emits `AuthorizationUsed(address indexed authorizer,
bytes32 indexed nonce)` from the token contract in the same transaction as the
`Transfer`. A `Transfer` whose transaction also carries an `AuthorizationUsed`
from that token is an authorised (x402-style) settlement; one without it is a
plain transfer. Both are reported; only the first supports any claim about x402
specifically.

### Limits this design does not overcome — state them in the report

* **"Not owner-funded" is implemented as two hops, not a graph.** `from` ≠ owner,
  plus the fleet-owner check. An owner routing funds through a fresh
  intermediary is counted as external. Full funding-graph tracing is not
  attempted, so **"external" is an upper bound on genuine earnings**.
* **Direction only, not purpose.** An external stablecoin transfer is not proof
  of a service rendered. Airdrops, refunds, and mistakes all look identical.
* **Zero means "nothing visible in scope"**, not "never earned anything".

---

## 6. The launch funnel

One funnel, every step a count the census can defend, no contested denominator:

| step | Base, run `cfbfcc01` |
|---|---:|
| agents registered on-chain | **60,097** |
| documents that resolve and parse (rung 3 `pass`) | **30,031** |
| documents declaring any service | **16,865** |
| documents declaring an `agentWallet` service entry *(convention, not spec)* | **920** |
| agents whose on-chain `getAgentWallet` is set *(the spec's mechanism)* | *§5 pending* |
| agents with ≥1 external incoming stablecoin transfer in scope | *pending* |
| …of those, arriving via `transferWithAuthorization` (x402) | *pending* |

Alongside it, not inside it, because they are claims about coherence rather than
steps in a pipeline:

* **1,977** declare `x402Support: true` and publish no address.
* **695** publish an address and declare `x402Support: false`.
* **409** publish an address the registry's verified value contradicts; **50**
  publish one it has never verified.
* **2** declare two different payment addresses with no rule for choosing.

## What is settled and what is not

Settled: the spec question (§1), the second denominator (§2), the 30,253
reconciliation (§3), the multi-wallet precedence question (§4), and the method
(§5).

Not yet run: the population-wide `getAgentWallet` count, and the transfer
analysis itself.
