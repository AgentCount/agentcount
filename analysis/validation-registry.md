# The Validation Registry — the last uncovered third of the standard

ERC-8004 has three registries. This census reads two of them. This document
measures the third, across all four swept chains at their pinned blocks.

**Result in one line: 23 agents out of 354,858 — 0.0065% — have ever been the
subject of a validation request, and 244,208 of them are on a chain where the
mechanism has never been used at all.**

---

## 1. Why this needed measuring rather than assuming

`chains.validation_registry` is `NULL` for every seeded chain, and
`crates/indexer/src/chains.rs` reads `NULL` as *"this registry is absent on
this chain"* — it skips the address entirely. `scripts/seed_chains.sql` states
the assumption in a comment:

> Base has an Identity and a Reputation registry but no Validation Registry —
> `validation_registry` is NULL, which the code reads as "this feature is
> absent on this chain".

The `validations` table exists in the schema and **contains zero rows**. So
the honest description of the census's prior position is not "the Validation
Registry is unused" but **"the Validation Registry has never been looked at"**.
Those are different claims, and only one of them was earned.

The curated upstream (`erc-8004/erc-8004-contracts`) supports the assumption —
it publishes Identity and Reputation addresses for 30+ networks and no
Validation Registry address anywhere, noting the contract is "still under
active update and discussion with the TEE community". **That is a claim on a
web page, so it was not taken as the answer.** It was tested against the
chains.

## 2. Method — search for the event, not for an address

Scanning a known address would have inherited exactly the assumption under
test. Instead this scans for **the spec's own event signatures by `topic0`
with no address filter**, which finds every contract on the chain emitting an
ERC-8004 validation event, whoever deployed it and wherever it lives:

```
ValidationRequest(address,uint256,string,bytes32)
  -> 0x530436c3634a98e1e626b0898be2f1e9980cc1bd2a78c07a0aba52d0a48a5059
ValidationResponse(address,uint256,bytes32,uint8,string,bytes32,string)
  -> 0xafddf629e874ccc3963b6a888c477bd464a6c8525024fc88759ea3b2326349ae
```

Both computed with `cast keccak` from the signatures in the pinned spec
(lines 362 and 380), not copied from a third party.

**This decision is what produced the finding.** An address-filtered scan of
`NULL` returns zero, and zero would have been reported as fact.

### Bound, stated because it limits the claim

Each chain is scanned from the block its canonical Identity Registry
(`0x8004a169…`) first has code, to the run's pinned block. Those deploy blocks
were unknown to this project — `deploy_block` is `0` for every seeded chain —
and were found by binary search on `eth_getCode`, with both sides of the
boundary verified (0 bytes at *deploy − 1*, 130 bytes at *deploy*):

| chain | Identity Registry deployed at | pinned block | blocks scanned |
|---|---:|---:|---:|
| base | 41,663,783 | 49,262,617 | 7,598,835 |
| bsc | 79,027,268 | 112,874,357 | 33,847,090 |
| mainnet | 24,339,871 | 25,640,407 | 1,300,537 |
| celo | 58,396,724 | 73,448,013 | 15,051,290 |

A validation event before that block cannot concern an agent in this census,
because the registry those agents live in did not yet exist. **Base, BSC and
mainnet were also scanned from block 0 in an earlier pass and returned
identical counts**, so for those three the bound is demonstrably not
load-bearing. Only Celo's result depends on it, because Celo's provider caps
`eth_getLogs` at 10,000 blocks (the other three accept unbounded ranges) and a
genesis scan there costs ~7,300 sequential calls.

## 3. What is actually there

| chain | agents | requests | responses | registries | validators | **agents validated** |
|---|---:|---:|---:|---:|---:|---:|
| base | 60,097 | 74 | 68 | 9 | 5 | **19** |
| bsc | 244,208 | 0 | 0 | 0 | 0 | **0** |
| mainnet | 40,806 | 0 | 0 | 0 | 0 | **0** |
| celo | 9,747 | 31 | 27 | 2 | 22 | **4** |
| **total** | **354,858** | **105** | **95** | **10** | **27** | **23** |

Ten distinct Validation Registry addresses exist across the two chains that
have any — `0x8004cc8439f36fd5f9f049d9ff86523df6daab58` appears on **both**
Base and Celo at the same address, so 9 + 2 = 10 distinct, not 11.

**The seed is wrong, and this is a disagreement to report rather than
silently patch.** A Validation Registry *is* deployed and *is* being used on
two of the four chains. What is true is the narrower statement that **no
canonical Validation Registry was deployed by the ERC-8004 team**; the
deployments that exist are third-party.

### They are wired to the registry this census sweeps

Every one of the nine Base deployments answers `getIdentityRegistry()` with
`0x8004A169FB4a3325136EB29fA0ceB6D2e539a432` — the exact Identity Registry
this census enumerates. So these validations concern **censused agents**, not
a parallel population.

On Celo, one of the two does. **The other, `0x279a126bcc…` (2,122 bytes),
reverts on `getIdentityRegistry()`.** The pinned spec (line 347) requires that
the identity registry address "is set via `initialize(address
identityRegistry_)` and is visible by calling `getIdentityRegistry()`". A
contract emitting `ValidationRequest`/`ValidationResponse` without exposing
that getter is **not a conformant Validation Registry by the spec's own
interface requirement**, and its 10 events are reported separately below
rather than pooled.

Most deployments are 130–170 bytes — proxies. One (`0xb28c303a…`, 3,547
bytes) is a full implementation.

## 4. What the validations say

### Base — 5 validators, 19 agents, one validator per registry

Every Base deployment has **exactly one** validator address, which is the
signature of a self-contained deployment by a single party rather than a
shared marketplace.

Response values, `uint8` on the spec's 0–100 scale:

| value | responses |
|---:|---:|
| 100 | 36 |
| 95 | 9 |
| 85 | 8 |
| 20 | 2 |
| 0 | 13 |

Tags: `capability_match` (24), `code-audit` (18), `tee-attestation` (12),
`vuln_scan` (11), `gdpr_compliance` (2), `paid` (1).

**43 of 68 responses carry an empty `responseURI`** — permitted, since the
field is optional. Of those that do carry one, several are of the form
`ipfs://QmAuditReport_Agent18425`. Stated without inference: **that is not a
resolvable IPFS CID.** A CIDv0 is `Qm` followed by 44 base58 characters;
this string is 24 characters and contains `_`, which is not in the base58
alphabet. Seven others are inline `data:application/json;base64,` URIs
decoding to `{"type":"tee-attestation…`.

### Celo — one agent accounts for two-thirds of it

Split by contract, the Celo picture is two unrelated things:

| registry | agents | requests | tags | values |
|---|---|---:|---|---|
| `0x8004cc84…` (conformant) | **1870 only** | 21 | `security-audit`, `service-execution`, `api-liveness`, `payment-verification`, `schema-compliance` | 98–100 |
| `0x279a126b…` (reverts) | 2, 3, 9378 | 10 | `delivered` | all 100 |

**Agent 1870 is Toppa** (`https://api.toppa.cc/registration.json`), and it is
the most-validated agent in the entire ERC-8004 population: 21 of the 105
validation requests ever made across four chains, from **22 distinct
validator addresses**, across five different tags. Nothing else in the census
looks like it — every Base registry has a single validator; this one agent has
22.

Agents 2 and 3 are the third and fourth agents ever minted on Celo.

## 5. What may and may not be said

**Supported:**

- 23 of 354,858 agents (0.0065%) have ever been the subject of a validation
  request; 105 requests and 95 responses exist in total.
- The mechanism has **never been used on BSC or Ethereum mainnet** — 285,014
  agents, 69% and 12% of the population, zero events.
- No canonical Validation Registry has been deployed; the ten that exist are
  third-party, and one of them does not implement the spec's required
  `getIdentityRegistry()`.
- One agent (Toppa, Celo #1870) accounts for 20% of all validation activity
  ever recorded.

**Not supported, and must not be claimed:**

- That validation is "unused" in the sense of *unwanted* — 105 requests is
  small but it is not zero, and the contract's own upstream describes it as
  still under design. A mechanism that has not been finalised has not been
  rejected.
- Any reading of what a validation *means*. A `response` of 100 tagged
  `api-liveness` is a claim by one address about an agent. This census does
  not re-execute the validation, does not know the validator, and takes no
  position on whether the claim is true. **The count is the finding; the
  verdicts are not evidence of anything except that they were written.**
- That the missing responses (105 requests vs 95 responses) are failures.
  Ten requests have no response at the pinned block. `validationResponse` may
  be called later, or never; absence at a block is not a verdict.

## 6. Reproduction

```sh
cast keccak "ValidationRequest(address,uint256,string,bytes32)"
cast keccak "ValidationResponse(address,uint256,bytes32,uint8,string,bytes32,string)"

# per chain, from the Identity Registry's deploy block to the run's pinned block
cast logs --from-block <deploy> --to-block <pinned> \
  --rpc-url "$RPC_URL_<CHAIN>" <topic0>

# and the test that makes a deployment ERC-8004 rather than merely event-shaped
cast call <registry> 'getIdentityRegistry()(address)' --rpc-url "$RPC_URL_<CHAIN>"
```
