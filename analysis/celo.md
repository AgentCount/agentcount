# Celo — one deployer's batch, not an ecosystem

Celo is the outlier on every measure this census takes. It attests at **79.5%**
against Base's 49.2%, mainnet's 4.1% and BSC's 1.8%, and only **1.6%** of its
agents have no way to be reached against 61–81% everywhere else. On the
published numbers it is the healthiest ERC-8004 deployment in existence.

**It is one platform. 87.7% of Celo's agents were minted by two addresses, and
99.8% of those agents' attestations were written by three.**

Run `7833fc49-a5b7-477b-99ce-946f650f0064`, pinned block 73,448,013, 9,747
agents.

---

## 1. Ownership

710 distinct owners hold 9,747 agents. Two of them hold **87.9%**:

| owner | agents | share |
|---|---:|---:|
| `0xc15366b9c611d23dd1433c5f6782b2ab64457d03` | 6,934 | 71.1% |
| `0xfb2ff4eb9eb00a9b019e4014bbc67c5c3adfa2c5` | 1,630 | 16.7% |
| next 708 owners combined | 1,183 | 12.1% |

Both are EOAs.

## 2. They were minted in consecutive batches, and they hand off

`agent_id` is the registry's mint counter, so id order is mint order. The two
owners occupy near-perfect contiguous ranges — and adjacent ones:

| owner | id range | agents | span | share of span |
|---|---|---:|---:|---:|
| `0xfb2ff4…` | 169 – 1,875 | 1,630 | 1,707 | **95.5%** |
| `0xc15366…` | 1,877 – 9,045 | 6,934 | 7,169 | **96.7%** |

The second owner's range begins **two ids after the first owner's ends**. In
maximal runs of consecutive ids, `0xc15366…` holds 110 blocks with a largest
single run of **1,503 consecutive agents**; `0xfb2ff4…` holds 16 blocks with a
largest run of 642. No other owner on the chain has a run longer than 72.

This is not a population arriving. It is a script running.

## 3. They are the same platform, and it says so

The agents' own registration documents carry non-spec `platform` and
`platformUrl` fields:

| `platform` | `platformUrl` | agents | distinct owners |
|---|---|---:|---:|
| `CeloNova` | `https://celonova.xyz` (8,546) / `https://api.celonova.xyz` (15) | 8,561 | 2 |
| *(absent)* | — | 1,172 | 706 |
| `TeliGent` | — | 9 | 1 |
| `celonova` (lowercase) | — | 3 | 1 |
| `aigora` | — | 2 | 2 |

**8,564 of 9,747 agents (87.9%) are CeloNova** — 8,561 spelled `CeloNova` plus
3 spelled `celonova` — and that set is *exactly* the two batch owners' 6,934 +
1,630. The platform self-identifies in its own public documents; this is not
inference.

*Spot check, 2026-07-30: `https://celonova.xyz` did not answer within 20s. One
request, from one network, outside the census's probe path — recorded as an
observation, not as a census result.*

## 4. The documents are generated

| group | documents | distinct top-level key shapes | share in modal shape |
|---|---:|---:|---:|
| `0xc15366…` | 6,934 | **1** | **100.0%** |
| `0xfb2ff4…` | 1,630 | 7 | 94.9% |
| everyone else | 886 | 125 | 41.1% |

Every one of `0xc15366…`'s 6,934 documents has an identical field set:

```
active, agentHash, birthHeadline, birthSource, description, did, image, name,
platform, platformUrl, registrations, services, supportedTrust, tags, type,
updatedAt, version, x402Support
```

The *content* varies — 6,514 distinct names and 6,934 distinct descriptions
("Nova Narrator", "Crimson Frequency", "Pulsar Prime", "Chronos Pump") — so
this is a generator with a template, not copy-paste.

The same shows in rung 4's SHOULD-gaps. CeloNova's 8,490 rung-4 passes produce
**3** distinct signatures, two of which cover 8,487 of them. The other 708
owners produce 20 signatures across 863 passes:

| group | signature | agents |
|---|---|---:|
| CeloNova | `services[].version` | 7,123 |
| CeloNova | `registrations, services[].version` | 1,364 |
| CeloNova | *(1 further signature)* | 3 |
| rest | `services[].version` | 467 |
| rest | `registrations, services[].version` | 168 |
| rest | *(18 further signatures)* | 228 |

Per rung-4 pass, CeloNova produces one signature per 2,830 agents; everyone
else, one per 43.

## 5. Why Celo looks reachable: it never leaves the chain

**90.9% of Celo's agents use an inline `data:` URI** — against 25.8% on Base,
23.6% on mainnet, 59.6% on BSC. And on every chain, an inline document passes
rung 2 at exactly **100%**:

| chain | inline `data:` URI | rung 2 pass | fetched over the network | rung 2 pass |
|---|---:|---:|---:|---:|
| base | 15,513 | **100.0%** | 44,584 | 36.3% |
| bsc | 145,665 | **100.0%** | 98,543 | 35.7% |
| mainnet | 9,613 | **99.9%** | 31,193 | 25.7% |
| celo | 8,856 | **100.0%** | 891 | 71.6% |

This is **correct behaviour, not a loophole**: the pinned spec explicitly
blesses it (line 52, and lines 192–195 recommend a base64 `data:` URI for
fully on-chain metadata). Rung 2 asks whether the registration document can be
retrieved, and for an inline document the answer is yes by construction.

But it means **rung 2's cross-chain differences are mostly a measure of how
many registrants inline their document, not of how many run a working server.**
Celo's 1.6% "no way to reach" is the arithmetic of 90.9% inlining.

## 6. Why Celo looks attested: three addresses

Celo's 7,745 attested agents (79.5%) split like this:

| group | attested | feedback entries | mean distinct authors | exactly one author |
|---|---:|---:|---:|---:|
| CeloNova | 6,836 | 11,906 | 1.50 | **75.3%** |
| everyone else | 909 | 15,626 | 4.41 | 67.4% |

Three EOAs wrote feedback for more than 2,000 agents each:

| client | agents | of which CeloNova |
|---|---:|---:|
| `0xf9946775891a24462cd4ec885d0d4e2675c84355` | 3,605 | 3,169 |
| `0xc71a15fcb1149254f97059f6cf3f6ed43990ebd4` | 3,533 | 3,411 |
| `0xa06f907f7ea437ebe60e3d452831ec69e5be43a4` | 2,111 | 2,005 |

> **6,825 of CeloNova's 6,836 attested agents — 99.8% — are attested by one or
> more of those three addresses.**

Remove CeloNova and Celo is 1,183 agents attesting at 76.8% — still high, but a
tenth the size, and carrying **more feedback volume** (15,626 entries) than the
6,836 batch agents do.

### The limit on this finding, stated plainly

None of the three clients is either batch owner, and the Reputation Registry
already forbids feedback from the agent's owner. **This census cannot tell
whether they are related parties.** The spec also bans feedback from an
approved ERC-721 operator (line 217), and **this census never reads
approvals** — so it cannot detect operator self-feedback, and nothing here
should be read as alleging it. What is measured is concentration, not identity:
three addresses, 99.8% coverage, no on-chain link to the owners either way.

## 7. Agent 1870 — Toppa, and the only thing on Celo that looks unlike the rest

Agent 1870 is **not** CeloNova (owner `0x558e7bfaf2…`, `agentURI`
`https://api.toppa.cc/registration.json`). It is the most externally-validated
agent in the entire 354,858-agent census:

- **581 feedback entries from 490 distinct authors** — against CeloNova's mean
  of 1.50 authors.
- **21 of the 105 ERC-8004 validation requests ever made across four chains**
  (see `validation-registry.md`), from **22 distinct validator addresses**,
  tagged `security-audit`, `service-execution`, `api-liveness`,
  `payment-verification`, `schema-compliance`, with responses of 98–100.

And the census could not read its document at all:

| rung | status | why |
|---|---|---|
| 1 `registered` | **pass** | |
| 2 `resolvable` | **error** | `robots_unavailable: timeout fetching robots.txt` |
| 3, 4, 5 | **skipped** | rung 2 did not answer |
| 7 `attested` | **pass** | 581 entries, 490 authors |

This is the ladder working as designed. Rung 2 recorded `error`, not `fail` —
the census could not ask, which is not the same as the agent failing — and
rung 7 is on its own track, so an unreadable document does not suppress a
reputation finding. The single most-validated agent in the ecosystem is one
whose registration document this census has never seen.

*Spot check, 2026-07-30: `https://api.toppa.cc/registration.json` returned
404. At sweep time the failure was a robots.txt timeout. Different failure,
different day; both observations, neither a census result.*

## 8. The answer to the question

**One deployer's batch.** Two addresses, consecutive mint ranges, one document
generator, two SHOULD-gap signatures, inline documents that make network
reachability moot, and three addresses supplying 99.8% of the attestations.

**What this does NOT support:** that CeloNova is fraudulent, or that its agents
do not work. A platform minting agents for its users on their behalf, hosting
their metadata on-chain, and running its own attestation service is a coherent
and legitimate product. Every fact above is equally consistent with that. The
finding is about **what the census's chain-level numbers mean**, not about
CeloNova's conduct:

> Celo's 79.5% attestation rate and 1.6% unreachability are not
> ecosystem-level facts. They are one platform's product decisions, measured at
> chain scale — and any per-chain comparison that does not say so is
> comparing a platform to four ecosystems.

## 9. Reproduction

All queries run against run `7833fc49-a5b7-477b-99ce-946f650f0064` in local
Postgres; the feedback client sets come from the `NewFeedback` event log
(`analysis/feedback-values.md` §1), which reconciles entry-for-entry with the
census's own stored `getSummary` counts on this chain (27,532 = 27,532).
