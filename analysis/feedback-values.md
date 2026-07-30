# What the feedback actually says

Rung 7 (`attested`) counts feedback entries. It has never read one. The
Reputation Registry returns a **value** with every entry, and the census
discards it: `chain::Reputation::feedback` calls
`getSummary(agentId, clients, "", "")`, keeps `count`, and drops
`summaryValue` and `summaryValueDecimals` on the floor.

This document reads the values, across all four chains, at the pinned blocks.

**Two results, and the second one is the bigger:**

1. **The automation reading completes.** 83.3% of Base's feedback entries carry
   their tag's single most common value, and the largest tag on any chain —
   **284,066 entries** — has exactly **one distinct value** across all of them.
2. **Base's published feedback statistics describe 33.6% of Base's feedback.**
   The single agent the run could not read holds 66.4% of every feedback entry
   ever written on that chain.

---

## 1. Method, and a departure from the brief — reported, not resolved silently

The instruction was "the same `getSummary` calls rung 7 already makes, but read
the score values". This scans the **`NewFeedback` event** instead. Three
reasons, and the third is decisive:

- `getSummary` returns **one aggregate per agent**, so it cannot show the
  distribution *within* an agent — which is exactly where the automation
  signature lives.
- It costs two RPC calls for each of ~43,316 attested agents; the event scan
  costs a few hundred calls in total.
- **`getSummary` can only be asked about agents the run knows about.** The
  finding in §3 below is invisible to it by construction.

```
NewFeedback(uint256 indexed agentId, address indexed clientAddress,
            uint64 feedbackIndex, int128 value, uint8 valueDecimals,
            string indexed indexedTag1, string tag1, string tag2,
            string endpoint, string feedbackURI, bytes32 feedbackHash)
  topic0 -> 0x6a4a61743519c9d648a14e6493f47dbe3ff1aa29e7785c96c8326a205e58febc
```

`topic0` computed with `cast keccak`, then **verified against a known-good
log** — Base block 41,688,962, agent 1199, value 100, tag `speed` — which
matches, field for field, the row the indexer had independently stored for the
same transaction.

### `value` has no fixed scale, so nothing is pooled across tags

The spec's own table gives `starred` on 0–100, `uptime` in hundredths of a
percent, `responseTime` in milliseconds, and `tradingYield` signed in tenths. A
distribution of raw values pooled across tags would be arithmetic performed on
unlike units. **Everything below is grouped by `tag1`**, and `valueDecimals` is
applied per entry.

## 2. The scan reconciles exactly with the census — which is what makes §3 credible

Comparing event-log counts against the `feedback_count` rung 7 stored, per
agent:

| chain | log entries | stored `getSummary` sum | agrees? |
|---|---:|---:|---|
| bsc | 29,507 | 29,507 | **exact** |
| celo | 27,532 | 27,532 | **exact** |
| mainnet | 3,209 | 3,209 | **exact** |
| base | 427,867 | 143,713 | **differs by 284,154** |

Per agent on Base, **29,568 of 29,571 agree exactly**. The entire difference is
two things, and both are fully accounted for:

```
143,713  (what rung 7 recorded)
+ 284,072  (one agent — see §3)
+      82  (revoked feedback, which getSummary omits by design)
= 427,867  (every NewFeedback event on Base)
```

The 82 are real `FeedbackRevoked` events, affecting exactly 2 agents — the same
2 agents whose per-agent counts disagree, by exactly 82 between them. Base is
the only chain with any revocations at all; bsc, mainnet and celo have zero.

**This reconciliation is the point.** Three chains match to the entry, and the
fourth's gap resolves to the digit. The method and the census agree, so where
they disagree the disagreement means something.

## 3. Base's largest feedback recipient is the one agent the run could not read

**Agent 25975 holds 284,072 feedback entries from 62 distinct clients — 66.4%
of every feedback entry ever written on Base.** It has no row in the run.

This is **not** a silent drop, and the sweeper behaved correctly. It
deliberately excludes agents whose chain read fails rather than recording a
`fail` about them — an RPC failure is the census's problem, not the agent's —
and it records the count durably in the run's export manifest:

```json
"agent_count": 60098, "swept": 60097, "unreadable": 1, "unwritable": 0
```

Every one of the four runs carries that record: base 1 unreadable, bsc 77,
mainnet 0, celo 0. The discipline held. **What was never done was joining that
`1` to anything.**

The failure was transient: `ownerOf(25975)` returns
`0x69747C4Ce6185d21A33b3BcdBa980d659600aC7b` both at the pinned block and now.
The agent was readable; one read failed.

**And the 77 unreadable BSC agents cost nothing** — BSC's log total matches its
stored sum exactly, so none of the 77 holds any feedback at all. The Base gap
is not a general property of unreadable agents. It is one agent, and it
happened to be the biggest.

### What must change in the report

Every Base feedback figure currently published is computed over 33.6% of the
chain's feedback events, and does not say so. The figures are **correct for the
population the run measured** and must be labelled that way, with the excluded
agent stated. This is a scope caveat, not a retraction: no published number
becomes wrong, but "feedback on Base" as a phrase becomes unsupportable without
the qualifier.

## 4. The values

### Base — one tag is two-thirds of the chain

| tag1 | entries | distinct values | modal value | share at modal |
|---|---:|---:|---:|---:|
| `miner-vouch` | 284,066 | **1** | `1` | **100.0%** |
| `trust` | 36,083 | 25 | `85` | 78.8% |
| `trustScore` | 10,653 | 73 | `26` | 3.9% |
| `contractRisk` | 10,648 | 26 | `81` | 20.0% |
| `counterparty` | 10,643 | 25 | `70` | 14.3% |
| `activity` | 10,641 | 84 | `0` | 11.2% |
| `longevity` | 10,635 | 79 | `58` | 14.7% |
| `starred` | 9,966 | 84 | `80` | 26.9% |

`miner-vouch` is agent 25975's traffic: **284,066 entries carrying the single
value `1`**, from 62 addresses. It is not a rating. It is a counter with a
signature attached.

### The cross-chain constants

Two tags appear on all four chains with the same value almost every time:

| tag | base | bsc | mainnet | celo |
|---|---|---|---|---|
| `trust` = 85 | 78.8% | 94.4% | 81.5% | 91.7% |
| `liveness` = 100 | — | 95.8% | 98.4% | 99.3% |

`reachable` on mainnet is `1` in 98.1% of its 268 entries. These are the
signatures of automated taggers writing the same verdict repeatedly, not of
independent parties forming a judgement.

### BSC — 104 addresses, 29,507 entries

BSC has **104 distinct feedback clients for the entire chain**, averaging 284
entries each across 4,336 agents. Six tags (`personality`, `knowledge`,
`timeline`, `relationship`, `stance`, `style`) each sit at value `70` for
55–70% of their entries.

Independently, from rung 7's own stored evidence: **89.1% of BSC's attested
agents have feedback from exactly one author** (mainnet 88.8%, celo 74.3%,
base 53.5%).

### Whole-population concentration

| chain | entries | distinct tags | share of entries at their tag's modal value | tags with only ONE distinct value |
|---|---:|---:|---:|---:|
| base | 427,867 | 570 | **83.3%** | 394 |
| celo | 27,532 | 239 | **78.1%** | 161 |
| bsc | 29,507 | 34 | **68.7%** | 21 |
| mainnet | 3,209 | 185 | **66.0%** | 147 |

## 5. What may and may not be said

**Supported:**

- Two-thirds to five-sixths of all feedback carries its tag's single most
  common value.
- The largest single tag in the dataset (284,066 entries) has exactly one
  distinct value.
- BSC's entire feedback population comes from 104 addresses.
- Base's published feedback statistics cover 33.6% of Base's feedback events.

**Not supported:**

- That repeated identical values are *fake*. An automated liveness prober
  writing `100` every time it succeeds is behaving correctly and honestly;
  identical values are what a working automated check looks like. The finding
  is that **the reputation layer is mostly machine-written**, not that it is
  dishonest.
- Any claim that a value *means* what its tag says. Nothing verifies that a
  `trust` of 85 reflects trust, and this census does not re-derive any value.
- Comparisons of values across tags. Different tags are different units.

## 6. Consequences to carry into the report

1. **Rung 7's `feedback_count` is sound** — it reconciles to the entry on three
   chains and to the digit on the fourth.
2. **Base's feedback figures need the 33.6% qualifier and agent 25975 named.**
3. **`unreadable` needs joining to the data, not just logging.** The manifest
   recorded it correctly for a year of runs; nobody asked what was in it.
4. The value distribution belongs in the report as its own finding: the
   attestation layer is largely one machine writing the same number.

## 7. Reproduction

```sh
cast keccak "NewFeedback(uint256,address,uint64,int128,uint8,string,string,string,string,string,bytes32)"
cast keccak "FeedbackRevoked(uint256,address,uint64)"

# per chain, Identity Registry deploy block -> run's pinned block
cast logs --from-block <deploy> --to-block <pinned> \
  --address 0x8004baa17c55a88189ae136b182e5fda19de9b63 \
  --rpc-url "$RPC_URL_<CHAIN>" <topic0>

# the agent the Base run could not read
cast call 0x8004a169fb4a3325136eb29fa0ceb6d2e539a432 'ownerOf(uint256)(address)' \
  25975 --block 49262617 --rpc-url "$RPC_URL_BASE"
```
