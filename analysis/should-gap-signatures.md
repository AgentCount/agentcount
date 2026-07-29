# SHOULD-gap signatures and the 259 remaining rung-4 failures

**Analysis only.** No check logic, crate code, or schema was touched to
produce this document. Every number below was computed directly against
archived response bodies (`http_archive.body`) for the reference run, using
the **current** classification in `spec/REQUIRED_FIELDS.md` (post P0 FIXES
1–3), not the stale `fields_missing` / `should_gaps` evidence stored on
`check_results` (which was written under the old rung-4 semantics).

## Run used

`1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a` — confirmed as the most recent
**completed** run (`finished_at` not null) via:

```sql
select run_id, chain, started_at, finished_at, agent_count
from runs
where finished_at is not null
order by finished_at desc limit 1;
-- => 1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a | base | ... | 60049
```

60,049 agents registered (rung 1). All queries below are scoped to this
`run_id` and `chain = 'base'` (the run's only chain).

## Classification applied

The current SHOULD bucket per `spec/REQUIRED_FIELDS.md` (7 checks, computed
per document):

- `type`, `name`, `description`, `image` — simple top-level presence.
- `services` (alias: `endpoints`, `services` wins if both present) —
  **absent** vs **empty** recorded as distinct gap labels
  (`services` / `services_empty`); non-empty is no gap.
- `registrations` — at least one entry; absent, null, non-array, or
  empty-array all collapse to one gap label (`registrations`).
- `services[].version` — one aggregated gap (`services[].version`) if *any*
  present service/endpoint entry lacks `version`.

A key is "present" only if it exists and is not JSON `null` — matching
`crates/checks/src/rung4_conformant.rs::is_present`. This logic was
re-implemented directly in SQL (see below) rather than read from stored
evidence, per the task's instruction.

## Method and validation

**Population.** SHOULD-gap signatures are computed over every agent whose
rung-3 (`parseable`) result is `pass` in the reference run. Rung 3's logic
is unaffected by P0 FIXES 1–3 (only rung 4 changed), so the stored rung-3
status is a trustworthy eligibility filter; nothing about the *content*
judgement below comes from stored evidence — every field presence check is
recomputed from the raw body.

```sql
-- Eligibility: rung 3 pass in the reference run
select count(*) from check_results
where run_id = '1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a' and rung = 3 and status = 'pass';
-- => 29811
```

**Guarding bad bodies.** `convert_from(body,'UTF8')::jsonb` is wrapped in a
`pg_temp` PL/pgSQL function (`safe_body_json`) that catches both the UTF‑8
decode step and the JSON parse step in nested `BEGIN … EXCEPTION WHEN
OTHERS` blocks, returning `NULL` + an error tag instead of aborting the
query on a bad row — mirroring the guard the task specifies. Run against
the full `http_archive` (60,049 rows, not gated on rung 3), this reproduces
the archive-wide malformed-body count exactly:

```
bad_utf8:  411   (invalid byte sequences — binary/HTML fragments served for a JSON URI)
bad_json: 5690   (non-JSON bodies: HTML error pages, "Payment Required" pages, etc.)
no_body: 23884   (agents whose rung-2 resolvable check never got a body at all)
```

These 6,101 (`411+5690`) "has body but doesn't parse" rows are **not** the
2,046 figure quoted in the task — that figure is rung 3's fail count, which
is gated on rung 2 (`resolvable`) passing first (bodies from *failed*
resolutions, e.g. archived 402/404 HTML pages, are still stored in
`http_archive` but never reach rung 3). Once the same gate is applied
(`check_results` rung 2 = pass, or equivalently rung 3 status IS NOT NULL),
`safe_body_json` reproduces rung 3's own numbers exactly: 29,811 pass,
2,046 fail, 11 error. This cross-check (not just trusting rung 3's stored
status) is what justifies using rung-3-pass as the eligibility filter above.

**Sanity check against the task's histogram.** The task states the SHOULD
gap-count distribution is `0:897, 1:3337, 2:12271, 3:3120, 4:10135, 5:36,
6:15`. Recomputing gap *count* (not yet signature) from the 29,811-document
population above, fresh from bodies, reproduces this exactly:

```sql
-- (full query in should_gaps.sql below)
select gap_count, count(*) from should_gap_docs group by 1 order by 1;
```

| gap_count | count |
|---|---|
| 0 | 897 |
| 1 | 3337 |
| 2 | 12271 |
| 3 | 3120 |
| 4 | 10135 |
| 5 | 36 |
| 6 | 15 |

All 29,811 rows parsed and classified cleanly; **zero** rows fell into the
"bad body" bucket (expected, since this population is already gated on
rung-3 = pass). This match against the task's independently-known histogram
is strong evidence the field-level recomputation below is correct.

**Full SQL** (the `pg_temp.safe_body_json` function, the CTE chain building
`should_gap_docs`, and its self-check) is reproduced in full in
[Appendix: SQL](#appendix-sql) at the end of this document.

---

## Investigation A — is the SHOULD-gap distribution bimodal because of templates?

### A1/A2 — top 20 signatures by document count, with distinct owners

```sql
select
  signature, gap_count, count(*) as documents,
  count(distinct owner) as distinct_owners,
  round(count(*)::numeric / count(distinct owner), 1) as docs_per_owner
from doc_owners   -- should_gap_docs joined to agent_snapshots on (run_id,chain,agent_id)
group by signature, gap_count
order by documents desc limit 20;
```

| # | signature (missing fields) | gaps | documents | % of 29,811 | distinct owners | docs/owner |
|---|---|---|---:|---:|---:|---:|
| 1 | `image,registrations,services,type` | 4 | 10,088 | 33.84% | 6,446 | 1.6 |
| 2 | `registrations,services[].version` | 2 | 3,994 | 13.40% | 2,604 | 1.5 |
| 3 | `registrations,type` | 2 | 3,255 | 10.92% | 3,052 | 1.1 |
| 4 | `registrations,services_empty` | 2 | 3,224 | 10.81% | 2,485 | 1.3 |
| 5 | `registrations,services,type` | 3 | 2,214 | 7.43% | 677 | 3.3 |
| 6 | `services[].version` | 1 | 1,952 | 6.55% | 659 | 3.0 |
| 7 | `image,services_empty` | 2 | 1,150 | 3.86% | 540 | 2.1 |
| 8 | *(no gaps)* | 0 | 897 | 3.01% | 686 | 1.3 |
| 9 | `services_empty` | 1 | 748 | 2.51% | 392 | 1.9 |
| 10 | `registrations` | 1 | 466 | 1.56% | 430 | 1.1 |
| 11 | `registrations,services` | 2 | 438 | 1.47% | 184 | 2.4 |
| 12 | `image,registrations,services[].version` | 3 | 395 | 1.33% | 94 | 4.2 |
| 13 | `image,registrations,services` | 3 | 288 | 0.97% | 181 | 1.6 |
| 14 | `image` | 1 | 170 | 0.57% | 14 | 12.1 |
| 15 | `registrations,services[].version,type` | 3 | 153 | 0.51% | 126 | 1.2 |
| 16 | `image,services[].version` | 2 | 134 | 0.45% | 97 | 1.4 |
| 17 | `image,registrations,services_empty` | 3 | 49 | 0.16% | 42 | 1.2 |
| 18 | `image,services` | 2 | 47 | 0.16% | 5 | 9.4 |
| 19 | `description,image,registrations,services[].version,type` | 5 | 29 | 0.10% | 11 | 2.6 |
| 20 | `image,registrations` | 2 | 24 | 0.08% | 14 | 1.7 |

Cumulative share: top 5 signatures = 76.40% of the population; top 10 =
93.88%; **top 20 (of only 41 distinct signatures observed across the whole
29,811-document population) = 99.68%**. Independent, unstructured authorship
would not concentrate this tightly into so few exact field-omission
patterns — this part of the bimodality claim is well supported.

### A3 — top owners overall, and which signature(s) they use

```sql
select owner, count(*) as docs, count(distinct signature) as distinct_sigs
from doc_owners group by owner order by docs desc limit 25;
```

| owner | docs | distinct signatures used | dominant signature | its share of owner's docs |
|---|---:|---:|---|---:|
| `0x4efb565a48af6ac3885a2e69bed8e760ce360bca` | 2,293 | 1 | `image,registrations,services,type` | 100% |
| `0x83a0c82bb1cdb8ffa6e775f84fda30a2993087a0` | 1,243 | 1 | `image,registrations,services,type` | 100% |
| `0x6ffa1e00509d8b625c2f061d7db07893b37199bc` | 659 | 5 | `image,services_empty` | 82.0% |
| `0x67722c823010ceb4bed5325fe109196c0f67d053` | 628 | 1 | `services[].version` | 100% |
| `0xbbb9d1d143d1f6ebb5387bb651880921ed938a02` | 320 | 2 | `registrations,services[].version` | 99.1% |
| `0x9f1045d983a6ac1faea82fe9314b47de73515d1a` | 265 | 2 | `registrations,type` | 75.8% |
| `0x820c5091b047b652888f6aa7e1ee615d99f7c8cd` | 243 | 2 | `registrations,services[].version` | 99.6% |
| `0x339559a2d1cd15059365fc7bd36b3047bba480e0` | 239 | 3 | `services[].version` | 98.7% |
| `0x40272e2eac848ea70db07fd657d799bd309329c4` | 144 | 3 | `image` | 97.2% |

**Baseline, for context.** Across the whole 29,811-document population
there are 18,514 distinct owners, average 1.61 docs/owner; 17,252 of them
(93.2%) publish exactly one document; only 127 owners publish 10+, only 13
publish 100+; the single largest owner in the whole population is
`0x4efb565a48af6ac3885a2e69bed8e760ce360bca` at 2,293 docs.

### A4 — owner concentration *per signature* (the load-bearing test)

The task specifies the distinguishing test explicitly: a signature spread
across a handful of owners is strong evidence of one tool; the same
signature spread across thousands of owners is weaker — a popular default.
Both patterns are present here, and they do not overlap.

```sql
-- for each top-20 signature: total docs, and the single largest owner's share
select signature, total_docs, max_owner_docs,
       round(100.0*max_owner_docs/total_docs, 1) as pct_from_top_owner
from ... group by signature order by total_docs desc;
```

| signature | documents | top single owner's share |
|---|---:|---:|
| `image,registrations,services,type` (**the #1 signature, 33.8% of the population**) | 10,088 | **22.7%** (2 owners together: 35.1%) |
| `registrations,services[].version` | 3,994 | 7.9% |
| `registrations,type` | 3,255 | 6.2% |
| `registrations,services_empty` | 3,224 | 3.2% |
| `registrations,services,type` | 2,214 | 1.7% |
| `services[].version` | 1,952 | **32.2%** |
| `image,services_empty` | 1,150 | **47.0%** |
| `image` | 170 | **82.4%** |
| `image,services` | 47 | **91.5%** |

**Reading this:**

- The single dominant signature, `image,registrations,services,type`
  (missing exactly `image`, `registrations`, `services`/`endpoints`, and
  `type`, while keeping `name` and `description`) — **10,088 documents,
  33.8% of the entire population, and 99.5% of the entire "4 gaps" peak
  bucket (10,135 documents)** — is used by **6,446 distinct owners**, at an
  average of 1.6 documents per owner. That average is *statistically
  indistinguishable from the population baseline* (1.61 docs/owner
  overall). The largest single owner using this exact signature accounts
  for only 22.7% of it (2,293/10,088); the top two owners together account
  for 35.1%. **This is not a small number of generators.** It reads as a
  widely-shared registration default — plausibly one SDK, template, or
  tutorial whose minimal example sets `name`/`description` but not the
  other four SHOULD fields — adopted independently by thousands of
  different owners, not one entity's output.
- By contrast, several *smaller* signatures show real single-owner
  concentration: `image` (170 docs, 82.4% from one owner,
  `0x40272e2eac848ea70db07fd657d799bd309329c4`), `image,services` (47 docs,
  91.5% from one owner), `image,services_empty` (1,150 docs, 47.0% from
  `0x6ffa1e00509d8b625c2f061d7db07893b37199bc`), and `services[].version`
  (1,952 docs, 32.2% — 628 of them — from `0x67722c823010ceb4bed5325fe109196c0f67d053`
  alone). These *are* consistent with a small number of individual
  operators or tools each producing a large, internally-uniform batch —
  but they are collectively a small share of the population (the four
  examples above sum to 3,319/29,811 = 11.1%).

### A4 verdict — does the data support the template hypothesis?

**Partially, and the two peaks are not the same kind of finding.**

- **The peak at 4 gaps (10,135 docs, 34.0% of the population) is explained
  almost entirely (99.5%) by one exact SHOULD-gap signature** — strong
  evidence of *a* fixed pattern (a shared default of some kind) — **but
  that pattern is spread across 6,446 distinct owners with no owner
  concentration above baseline.** This is evidence of a **popular shared
  template/SDK default**, not evidence of a **small number of sybil-like
  generators**. Those are the two hypotheses the task asks to be
  distinguished, and the evidence points to the first, not the second.
- **The peak at 2 gaps (12,271 docs, 41.2%) is not one pattern at all** —
  it is a near-even split across (at least) three comparably-sized, mutually
  distinct signatures (`registrations,services[].version`: 3,994;
  `registrations,type`: 3,255; `registrations,services_empty`: 3,224 — plus
  smaller contributors), each of which is *also* broadly owner-distributed
  (2,604 / 3,052 / 2,485 distinct owners respectively, i.e. essentially
  1–1.5 docs/owner, at or below the population baseline). So the "2" peak
  is the sum of several independently-popular field-omission patterns, not
  one template either.
- **The trough at 3 gaps (3,120 docs, 10.5%) is not an absence of
  templates** — its own largest signature (`registrations,services,type`,
  2,214 docs, 71% of the bucket) is exactly as broadly owner-distributed as
  everything else here (677 owners, top owner 1.7%). It is simply a less
  common combination arithmetically (it requires *not* having exactly the
  right combination of the more-popular 2-gap and 4-gap patterns).
- **A small number of signatures (4 of the top 20, ~11% of the population)
  genuinely are owner-concentrated** (47–92% from a single address) and are
  the closest thing here to "one tool, thousands of copies" in the sybil
  sense the task warns about — but they are minor contributors next to the
  two dominant, broadly-shared patterns that explain most of the
  bimodality.

**Stated as the task requires, with denominators:** of 29,811 documents,
41 distinct SHOULD-gap signatures exist; the top 20 account for 29,715
(99.68%). The single largest signature (10,088 docs, 33.84% of the
population) shows a docs-per-owner ratio (1.6) matching the population
baseline (1.61) — i.e., **no measurable owner concentration** — while a
handful of much smaller signatures (collectively 3,319 docs, 11.1% of the
population) show 47–92% concentration in one owner each. **The data
supports "a small number of shared templates/SDK defaults, used broadly by
independent registrants" for the bulk of the bimodality, and "a handful of
individual high-volume tools" for a minority (~11%) of it — not "a
handful of generators account for most of the population," which the raw
signature-count alone might have suggested before checking owner
concentration.**

---

## Investigation B — the 259 remaining rung-4 failures

Recomputed from scratch, not read from stored `check_results`. The current
rung-4 MUST rule (`spec/REQUIRED_FIELDS.md` §MUST /
`rung4_conformant.rs::REGISTRATION_ENTRY_FIELDS`): when a `registrations`
array is present and non-empty, every entry in it must have both `agentId`
and `agentRegistry` (non-null); any entry missing either is a MUST
violation and fails the document.

```sql
-- (full query in investigation_b.sql, Appendix)
select count(distinct agent_id) as failing_agents, count(*) as violation_rows
from must_fail_docs;
```

| failing_agents | violation rows (an agent can have >1 bad entry) |
|---:|---:|
| **259** | 322 |

This reproduces the addendum's "259 remaining rung-4 failures" figure
exactly, computed independently from bodies rather than trusted from the
stale evidence.

### B1 — owner concentration

```sql
select s.owner, count(distinct m.agent_id) as agents
from must_fail_docs m join agent_snapshots s using (agent_id, ...)
group by s.owner order by agents desc;
```

161 distinct owners among the 259 failing agents. Highly uneven:

| owner | failing agents | % of 259 |
|---|---:|---:|
| `0xd3d03f57c60bbefe645cd6bb14f1ce2c1915e898` | 54 | **20.8%** |
| `0xa29d618d9bfb1d4d626e915cb3701731974e5b26` | 11 | 4.2% |
| `0x52ce108ae72712929fd2b8c6177a9b39e0ccf644` | 10 | 3.9% |
| `0x58554e8423ef5c10be6ffc82efaba9149f64de3d` | 6 | 2.3% |
| `0xe7dddffc6e55c34199b2f79630e6a8d433c74260` | 5 | 1.9% |
| `0xf51fc5849f5dd081bed60d9a3a5f17b0b9a309b8` | 5 | 1.9% |
| `0x7f28a01efa5151e7529c109f90a77ee4bd1eb6ff` | 5 | 1.9% |
| `0x8a8846ef65f7e5b37225f7b4ca500fce9c5688c8` | 4 | 1.5% |
| 4 more owners | 2–3 each | remainder |

Exact breakdown of all 161 owners: **1 owner** with 54 failing agents (the
cluster above); **12 owners** with 2–11 failing agents each, totaling **57
agents**; **148 owners** with exactly **1** failing agent each. Those 148
single-agent owners include the 48 `api.orbit-agents.com` owners discussed
in B2 below (leaving 100 single-agent owners on other hosts/schemes).
1 + 12 + 48 + 100 = 161 owners; 54 + 57 + 48 + 100 = 259 agents.

The top owner alone accounts for over a fifth of all current MUST
failures. Every one of that owner's 54 failing documents shares the
*identical* top-level key set (`{active, description, image, name,
registrations, services, supportedTrust, type, x402Support}` — checked via
`jsonb_object_keys`), and every one has `registrations[0].agentId: null`
against the same `agentRegistry` value
(`eip155:8453:0x8004A169FB4a3325136EB29fA0ceB6D2e539a432`). Sampled names
are self-evidently test fixtures: `TestBot` (×3 near-duplicates, one with
the key typo `namE`), `OnChainTestBot`, `FullFlowTestAgent`, `Research
Agent` (used twice, verbatim, for different agents), `Social Agent
Survival Test`, `Devloper Agent Test`. Every sampled document's `image`
and `services[].endpoint` URLs point at `automaton.cloud` — this looks
like one hosting/agent-deployment platform generating many demo/test
agents whose registration payload always leaves `agentId` unset (plausibly
because the tool template-fills `agentRegistry` but doesn't yet know its
own freshly-minted `agentId` at generation time).

### B2 — missing field, and tokenURI scheme/host

```sql
select missing_agent_id, missing_agent_registry, count(*), count(distinct agent_id)
from must_fail_docs group by 1,2;
```

| missing field(s) | violation rows | agents | % of 259 |
|---|---:|---:|---:|
| `agentId` only | 189 | 187 | 72.2% |
| `agentRegistry` only | 113 | 55 | 21.2% |
| both | 20 | 17 | 6.6% |

`agentId`-missing dominates by a wide margin — consistent with the
"registry known, own tokenId not yet known at generation time" pattern
above.

tokenURI scheme/host:

| scheme / host | agents | % of 259 |
|---|---:|---:|
| `data:` (inline) | 170 | 65.6% |
| `https://api.orbit-agents.com` | 48 | 18.5% |
| `ipfs://` | 16 | 6.2% |
| `https://gateway.pinata.cloud` | 9 | 3.5% |
| `https://richard-hobbs.com` | 6 | 2.3% |
| 8 other distinct hosts | 1–2 each | remainder |

**`api.orbit-agents.com` is the second major cluster, and a genuinely
different kind of one, structurally.** All 48 agents whose tokenURI
resolves there are from **48 different owner addresses** (1:1 — no owner
concentration at all), and *every single one* of them fails with the
*same* missing field: `agentRegistry` (never `agentId`) — this single host
accounts for 48 of the 55 agents (87.3%) in the "`agentRegistry`-only
missing" bucket. This is a third-party agent-hosting/registration service
used independently by 48 different registrants, whose generated documents
consistently omit `agentRegistry` — a service-level bug shared across many
unrelated owners, not one owner's repeated behavior.

Together, these two identified clusters — the 54-agent single-owner
`automaton.cloud`-style cluster and the 48-agent, 48-owner
`api.orbit-agents.com` cluster — account for **102 of 259 failing agents
(39.4%)**, via two structurally different mechanisms (one operator running
one tool, vs. one hosting service used by many independent operators).

### B3 — sample of 10 raw bodies

Ten failing documents sampled directly (agent_id, owner, name, failure):

| agent_id | owner | tokenURI scheme | name | what's wrong |
|---:|---|---|---|---|
| 1 | `0x89e9...5029` | `data:` | ClawNews | 2-entry `registrations`; one complete (chain 56), one `agentId: null` (chain 8453, its own chain) |
| 2344 | `0x75b5...94adc` | `ipfs://` | ClawdMint | 20-entry multi-chain `registrations` list; 17 complete, 3 with `agentId: null` for chains it apparently hasn't registered on yet |
| 2461 | `0x6dde...c2583` | `data:` | Agents (MECHAIS mint agent) | single entry, `agentId: null`, `image: ""` (also should_gap-empty) |
| 15305 | `0x40db...3f942` | `data:` | Momo 🍑 | single entry, `agentRegistry` present, `agentId` key present but `null`; document also carries non-spec fields (`wallet`, `creator`, `createdAt`, `capabilities`) |
| 16633, 16636, 16638, 16642 | `0x8a88...688c8` | `data:` | "TestBot" / "Testy" (explicit test fixtures, one with description "for validating the claim flow") | all `agentId: null`, same `agentRegistry` |
| 16800 | `0x5d69...6134` | `data:` | cameriere | `registrations` entry has neither `agentId` nor `agentRegistry` — instead `{"network":"base","notes":"..."}`, a free-text description of intent rather than the spec's structured fields |
| 16895 | `0x3862...2d3eb` | `data:` | irvinecold | single entry, `agentId: null`, `agentRegistry` points at a *different* contract address than the canonical registry seen elsewhere |

**Characterisation.** These are overwhelmingly real, differently-branded
agents (ClawNews, ClawdMint, "Momo", "cameriere", "irvinecold") built by
different people/teams, not one obfuscated source — except for the
`0x8a88...688c8` and `automaton.cloud`-style clusters, which are
self-labelled test/demo fixtures. The dominant defect shape across the
sample is a `registrations` entry that is *structurally* present (the doc
author clearly knows the field exists and what shape it takes — most
entries correctly supply `agentRegistry`) but supplies `agentId: null`
rather than omitting the entry or the field outright — consistent with
tooling that fills the entry at mint time before the tokenId is known, or
that pre-declares intent to register on other chains it hasn't reached
yet (see agent 2344's 3 not-yet-registered chains). One entry (agent
16800) doesn't use the spec's schema at all, supplying prose (`notes`)
instead.

### B4 — verdict: one tool, several tools, or independent errors?

**Several, not one, and not simply independent either.** The 259 failures
decompose into:

1. **One identifiable single-operator tool**: 54 agents (20.8%), one
   owner, one platform (`automaton.cloud`-hosted content, uniform doc
   shape), one consistent bug (`agentId` always null).
2. **One identifiable shared hosting service**: 48 agents (18.5%), 48
   distinct owners, uniform failure mode (`agentRegistry` always missing) —
   a service-level bug affecting many unrelated registrants, not one
   owner's output.
3. **A tail of smaller multi-agent operators**: exactly 12 owners with 2–11
   failing agents each, 57 agents total (22.0% of the 259) — too small a
   sample per owner to characterise individually, but each owner's
   documents plausibly share one internal tool given the pattern seen in
   the larger two clusters.
4. **A long tail of apparently-independent single-agent failures**: 100
   owners (of the 148 single-agent owners, after excluding the 48 on
   `api.orbit-agents.com`) with exactly one failing agent each — 100/259 =
   38.6% — spread across a genuine variety of hosts, doc shapes, and even
   schema deviations (agent 16800's free-text `notes` field). These read as
   independent implementation mistakes by different teams hitting the same
   easy-to-miss requirement (an `agentId` that literally cannot be known
   until after the on-chain mint that creates it), not a shared tool.

So: **not "one broken tool"** (only accounts for 20.8%), **not "entirely
independent errors"** either (39.4% traces to exactly two identifiable
sources), but a mix dominated by two identifiable clusters plus a
genuinely long, independent tail — which is itself a finding worth
reporting precisely rather than rounding to either extreme.

---

## Appendix: SQL

The two scripts below are the actual queries run to produce every number
above (paths as used; adjust for your own checkout). Both assume
`DATABASE_URL` points at the Postgres instance holding the archived run.

### `should_gaps.sql` — builds `should_gap_docs`, the Investigation A base table

```sql
CREATE OR REPLACE FUNCTION pg_temp.safe_body_json(b bytea, OUT doc jsonb, OUT err text) AS $$
DECLARE
  txt text;
BEGIN
  err := NULL; doc := NULL;
  IF b IS NULL THEN err := 'no_body'; RETURN; END IF;
  BEGIN
    txt := convert_from(b, 'UTF8');
  EXCEPTION WHEN OTHERS THEN err := 'bad_utf8'; RETURN;
  END;
  IF left(txt,1) = chr(65279) THEN txt := substring(txt from 2); END IF; -- strip BOM, rung3 parity
  BEGIN
    doc := txt::jsonb;
  EXCEPTION WHEN OTHERS THEN err := 'bad_json'; RETURN;
  END;
END;
$$ LANGUAGE plpgsql;

CREATE TEMP TABLE should_gap_docs AS
WITH eligible AS (
  SELECT ha.agent_id, ha.body
  FROM http_archive ha
  JOIN check_results cr
    ON cr.run_id = ha.run_id AND cr.chain = ha.chain AND cr.agent_id = ha.agent_id
   AND cr.rung = 3 AND cr.status = 'pass'
  WHERE ha.run_id = '1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a'
),
parsed AS (
  SELECT e.agent_id, (safe_body_json(e.body)).* FROM eligible e
),
fields AS (
  SELECT
    p.agent_id, p.doc, p.err,
    (p.doc->'type'        IS NOT NULL AND p.doc->'type'        <> 'null'::jsonb) AS has_type,
    (p.doc->'name'        IS NOT NULL AND p.doc->'name'        <> 'null'::jsonb) AS has_name,
    (p.doc->'description' IS NOT NULL AND p.doc->'description' <> 'null'::jsonb) AS has_description,
    (p.doc->'image'       IS NOT NULL AND p.doc->'image'       <> 'null'::jsonb) AS has_image,
    CASE WHEN p.doc->'services' IS NOT NULL AND p.doc->'services' <> 'null'::jsonb THEN p.doc->'services'
         WHEN p.doc->'endpoints' IS NOT NULL AND p.doc->'endpoints' <> 'null'::jsonb THEN p.doc->'endpoints'
         ELSE NULL END AS active_services_value,
    CASE WHEN p.doc->'registrations' IS NOT NULL AND p.doc->'registrations' <> 'null'::jsonb
          AND jsonb_typeof(p.doc->'registrations') = 'array'
         THEN p.doc->'registrations' ELSE NULL END AS registrations_entries
  FROM parsed p
),
computed AS (
  SELECT
    f.agent_id, f.err, f.has_type, f.has_name, f.has_description, f.has_image,
    CASE
      WHEN f.active_services_value IS NULL THEN 'absent'
      WHEN jsonb_typeof(f.active_services_value) = 'array'
           AND jsonb_array_length(f.active_services_value) = 0 THEN 'empty'
      WHEN jsonb_typeof(f.active_services_value) = 'array' THEN 'present'
      ELSE 'empty'
    END AS services_status,
    CASE
      WHEN f.active_services_value IS NOT NULL AND jsonb_typeof(f.active_services_value) = 'array'
      THEN EXISTS (
        SELECT 1 FROM jsonb_array_elements(f.active_services_value) elt
        WHERE NOT (elt->'version' IS NOT NULL AND elt->'version' <> 'null'::jsonb)
      )
      ELSE FALSE
    END AS version_gap,
    COALESCE(jsonb_array_length(f.registrations_entries), 0) AS registrations_checked
  FROM fields f
),
signed AS (
  SELECT c.agent_id, c.err,
    ARRAY(
      SELECT x FROM (VALUES
        (CASE WHEN NOT c.has_type THEN 'type' END),
        (CASE WHEN NOT c.has_name THEN 'name' END),
        (CASE WHEN NOT c.has_description THEN 'description' END),
        (CASE WHEN NOT c.has_image THEN 'image' END),
        (CASE WHEN c.services_status = 'absent' THEN 'services' END),
        (CASE WHEN c.services_status = 'empty' THEN 'services_empty' END),
        (CASE WHEN c.version_gap THEN 'services[].version' END),
        (CASE WHEN c.registrations_checked = 0 THEN 'registrations' END)
      ) v(x) WHERE x IS NOT NULL ORDER BY x
    ) AS should_gaps
  FROM computed c
)
SELECT s.agent_id, s.err, s.should_gaps,
       array_length(s.should_gaps, 1) AS gap_count,
       array_to_string(s.should_gaps, ',') AS signature
FROM signed s;

-- Join to owners for A1–A4:
CREATE TEMP TABLE doc_owners AS
SELECT g.agent_id, g.signature, g.gap_count, s.owner
FROM should_gap_docs g
JOIN agent_snapshots s
  ON s.run_id = '1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a' AND s.chain = 'base' AND s.agent_id = g.agent_id
WHERE g.err IS NULL;
```

### `investigation_b.sql` — builds `must_fail_docs`, the Investigation B base table

```sql
CREATE TEMP TABLE must_fail_docs AS
WITH eligible AS (
  SELECT ha.agent_id, ha.body
  FROM http_archive ha
  JOIN check_results cr
    ON cr.run_id = ha.run_id AND cr.chain = ha.chain AND cr.agent_id = ha.agent_id
   AND cr.rung = 3 AND cr.status = 'pass'
  WHERE ha.run_id = '1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a'
),
parsed AS (
  SELECT e.agent_id, (safe_body_json(e.body)).* FROM eligible e
),
regs AS (
  SELECT p.agent_id, p.doc,
    CASE WHEN p.doc->'registrations' IS NOT NULL AND p.doc->'registrations' <> 'null'::jsonb
          AND jsonb_typeof(p.doc->'registrations') = 'array'
         THEN p.doc->'registrations' ELSE NULL END AS registrations_entries
  FROM parsed p WHERE p.doc IS NOT NULL
),
violations AS (
  SELECT r.agent_id, r.doc, idx - 1 AS entry_index, entry,
    NOT (entry->'agentId' IS NOT NULL AND entry->'agentId' <> 'null'::jsonb) AS missing_agent_id,
    NOT (entry->'agentRegistry' IS NOT NULL AND entry->'agentRegistry' <> 'null'::jsonb) AS missing_agent_registry
  FROM regs r,
       LATERAL jsonb_array_elements(r.registrations_entries) WITH ORDINALITY AS t(entry, idx)
  WHERE r.registrations_entries IS NOT NULL
)
SELECT agent_id, doc, entry_index, entry, missing_agent_id, missing_agent_registry
FROM violations
WHERE missing_agent_id OR missing_agent_registry;
```

Owner concentration (B1), missing-field breakdown (B2a), and tokenURI
host/scheme (B2b/B2c) are straightforward `GROUP BY` queries against
`must_fail_docs` joined to `agent_snapshots`, shown inline in the relevant
sections above.
