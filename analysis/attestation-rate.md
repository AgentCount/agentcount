# Is 49.2% attestation genuine broad adoption, or concentrated activity?

**Analysis only.** No check logic, crate code, or schema was touched to
produce this document. All queries run directly against `check_results`,
`agent_snapshots`, and `runs` for the reference run; the one number that
cannot come from stored data (author concentration, Q3) was read live from
the Reputation Registry contract via `cast call`, pinned to the run's block.

## Run used

`cfbfcc01-fdaf-409f-9bed-abf706d865c7` — Base, pinned block **49,262,617**,
60,097 agents (confirmed against `runs`):

```sql
select run_id, chain, agent_count, started_at, finished_at
from runs where run_id = 'cfbfcc01-fdaf-409f-9bed-abf706d865c7';
-- => base | 60097 | 2026-07-29 10:43:02+02 | 2026-07-29 13:51:12+02
```

Rung 1 (`registered`) passes for all 60,097 agents, and rung 7 (`attested`)
has exactly one row per agent (60,097 = 29,570 pass + 30,527 fail) — the
ungated track described in `crates/checks/src/rung7_attested.rs`'s module
doc, confirming the headline figure under scrutiny:

```sql
select status, count(*) from check_results
where run_id = 'cfbfcc01-fdaf-409f-9bed-abf706d865c7' and rung = 7
group by status order by status;
--  fail  | 30527
--  pass  | 29570   →  29570 / 60097 = 49.2%
```

## 1. `feedback_count` distribution among attested agents (stored data, all 29,570)

```sql
select
  count(*) as n,
  min((evidence->>'feedback_count')::bigint) as min_fc,
  percentile_cont(0.5) within group (order by (evidence->>'feedback_count')::bigint) as median_fc,
  avg((evidence->>'feedback_count')::bigint) as mean_fc,
  percentile_cont(0.9) within group (order by (evidence->>'feedback_count')::bigint) as p90_fc,
  percentile_cont(0.99) within group (order by (evidence->>'feedback_count')::bigint) as p99_fc,
  max((evidence->>'feedback_count')::bigint) as max_fc
from check_results
where run_id = 'cfbfcc01-fdaf-409f-9bed-abf706d865c7' and rung = 7 and status = 'pass';
```

| n | min | median | mean | p90 | p99 | max |
|---|-----|--------|------|-----|-----|-----|
| 29,570 | 1 | **2** | 4.86 | 10 | 21 | **12,129** |

Heavily right-skewed: half of attested agents have 2 or fewer feedback
entries total, but the maximum (agent 55985) has 12,129 — nearly 3× the sum
you'd get from putting every other agent at the median. This alone is a
"many agents, one review each, plus a long tail" shape, not "everyone got
roughly equal, independent attention."

## 2. `distinct_authors` distribution among attested agents (stored data, all 29,570)

```sql
select
  count(*) as n,
  percentile_cont(0.5) within group (order by (evidence->>'distinct_authors')::bigint) as median_da,
  avg((evidence->>'distinct_authors')::bigint) as mean_da,
  percentile_cont(0.9) within group (order by (evidence->>'distinct_authors')::bigint) as p90_da,
  percentile_cont(0.99) within group (order by (evidence->>'distinct_authors')::bigint) as p99_da,
  max((evidence->>'distinct_authors')::bigint) as max_da
from check_results
where run_id = 'cfbfcc01-fdaf-409f-9bed-abf706d865c7' and rung = 7 and status = 'pass';
```

| n | median | mean | p90 | p99 | max |
|---|--------|------|-----|-----|-----|
| 29,570 | **1** | 2.25 | 3 | 10 | **8,754** |

```sql
select (evidence->>'distinct_authors')::bigint as da, count(*) as n
from check_results
where run_id = 'cfbfcc01-fdaf-409f-9bed-abf706d865c7' and rung = 7 and status = 'pass'
group by da order by da limit 5;
--  1 | 15806   (53.4% of all attested agents)
--  2 |  8418
--  3 |  3424
--  4 |  1072
--  5 |   232
```

**53.4%** of attested agents (15,806 / 29,570) pass on the strength of a
*single* client address. Of those, 10,817 (36.6% of all attested agents)
have exactly one feedback entry from exactly one author — the minimal
possible way to clear this rung. The median attested agent was vouched for
by exactly one address, once or twice. This is already a reason to look
harder at who that one address typically is — which stored evidence cannot
answer (it stores counts, not identities), so that question moves to the
live sample below.

## 3. Author concentration — from a live sample (the key question)

**This is not answerable from stored data.** `check_results` rung-7 evidence
holds `feedback_count` and `distinct_authors` — counts, not the client
addresses themselves — so whether the same handful of addresses recur
across thousands of agents cannot be determined by any query against
`check_results`. Answering it requires calling `getClients(agentId)` on the
Reputation Registry directly.

### Method

- **Population sampled:** the 29,570 agents with rung-7 `status = 'pass'` in
  this run.
- **Draw:** `setseed(0.42)` then `order by random() limit 300` inside a
  single Postgres session (deterministic given the seed and session, not
  claimed portable across Postgres versions/architectures — the query is
  reproducible in spirit, not bit-for-bit guaranteed):

  ```sql
  select setseed(0.42);
  select agent_id from check_results
  where run_id = 'cfbfcc01-fdaf-409f-9bed-abf706d865c7' and rung = 7 and status = 'pass'
  order by random()
  limit 300;
  ```

- **Sample size:** 300 agents (≈1.0% of the 29,570 attested population).
- **Read:** for each sampled agent, `cast call` against the Reputation
  Registry (`0x8004baa17c55a88189ae136b182e5fda19de9b63`) on Base, pinned to
  block **49,262,617** — the exact block this run pinned every other read
  to — replicating `chain::Reputation::feedback`'s live logic
  (`getClients(uint256) returns (address[])`) without modifying any crate
  code:

  ```sh
  cast call 0x8004baa17c55a88189ae136b182e5fda19de9b63 \
    "getClients(uint256)(address[])" <agent_id> \
    --rpc-url "$RPC_URL_BASE" --block 49262617
  ```

  Sequential, one call at a time, with retry/backoff on throttling
  (`429` / "compute units per second" — the same signature
  `crates/chain`'s `is_throttled` checks for), matching the task brief's
  "keep concurrency low and be patient." **All 300 reads succeeded — zero
  agents in the sample failed to read, no retries were exhausted.**

- **Validation:** every one of the 300 live `getClients` reads was compared
  against this run's own stored `distinct_authors` evidence for the same
  agent. **All 300 matched exactly** (0 mismatches) — confirming the sample
  is reading the same pinned state the run judged, not a post-hoc drifted
  chain state.

### Result

573 (agent, author-address) pairs across 300 agents, 124 distinct addresses.

| Address (truncated) | Sampled agents it appears for | Share of the 300-agent sample |
|---|---|---|
| `0xf653…807e` | **143** | **47.7%** |
| `0xc71a…ebd4` | 122 | 40.7% |
| `0x7c0a…ab78` | 70 | 23.3% |
| `0x718b…d136` | 42 | 14.0% |
| `0xa06f…43a4` | 42 | 14.0% |
| `0x809d…7c7` | 18 | 6.0% |
| (118 more addresses) | ≤5 each | ≤1.7% each |

86.3% of the 124 addresses seen (107 of them) appear for exactly **one**
sampled agent — a long tail of one-off authors sits alongside a small set of
addresses that recur constantly. That shape mirrors Q1/Q2's skew almost
exactly, which is what you'd expect if the same few addresses driving the
stored `feedback_count`/`distinct_authors` tails are, in fact, the same
addresses recurring here.

**The single strongest number:** one address, `0xf653…807e`, appears as a
feedback author for **143 of the 300 sampled agents (47.7%)**. Treating the
300-agent draw as a simple random sample of the 29,570-agent attested
population, the Wilson 95% confidence interval on that share is **42.1% –
53.3%**, i.e. an estimated **12,400 – 15,800 attested agents** — roughly
**42–53% of everyone this census counted as "attested"** — carry feedback
from this one address alone.

Widening to the top 6 addresses: **274 of 300 sampled agents (91.3%,** 95%
CI 87.6–94.0%) have at least one of these 6 addresses among their authors,
and **263 of 300 (87.7%,** 95% CI 83.5–90.9%) have **no other author at
all** — every single feedback entry on those agents traces back to one of
just 6 addresses. Extrapolated: an estimated **24,700 – 26,900 of the
29,570 attested agents** owe their `attested: pass` status entirely to 6
client addresses.

### Caveats on this estimate

- n=300 is a sample, not a census of authorship — the confidence intervals
  above are the honest uncertainty band, not a point fact the way the
  stored-data numbers in §1/§2 are.
- The sample was drawn once, with one fixed seed; it was not repeated with
  independent draws to check draw-to-draw stability (a legitimate follow-up
  if this number is going to anchor a published claim).
- This measures *how many agents an address left feedback for*, not
  feedback *value*, timing, or funding relationships between addresses —
  consistent with rung 7's own scope (see its module doc: sybil/coordination
  analysis is explicitly out of scope for this rung and not attempted here
  either).

## 4. Correlation with ownership

```sql
with attested as (
  select agent_id from check_results
  where run_id = 'cfbfcc01-fdaf-409f-9bed-abf706d865c7' and rung = 7 and status = 'pass'
)
select s.owner, count(*) as total_agents,
       count(*) filter (where a.agent_id is not null) as attested_agents
from agent_snapshots s
left join attested a on a.agent_id = s.agent_id
where s.run_id = 'cfbfcc01-fdaf-409f-9bed-abf706d865c7' and s.chain = 'base'
group by s.owner
order by total_agents desc
limit 15;
```

| owner (top by total agents owned) | total agents | attested |
|---|---|---|
| `0x4efb…0bca` | 2,293 | 2 (0.1%) |
| `0x4602…8101` | 1,613 | 863 (53.5%) |
| `0x83a0…87a0` | 1,243 | 0 (0.0%) |
| `0x3c08…83d8` | 968 | 131 (13.5%) |
| `0x6ffa…99bc` | 690 | 250 (36.2%) |
| `0x6772…d053` | 633 | 490 (77.4%) |
| `0xcc61…8062` | 500 | 319 (63.8%) |

Ownership is **not concentrated at the top in aggregate** — 15,698 distinct
owners hold at least one attested agent, and the 10 owners with the most
attested agents account for only **10.5%** of all 29,570 attested agents
(3,111 of them). But *conditional* attestation rate varies enormously and
non-randomly by owner: some owners of 1,000+ agents have essentially
**zero** attested agents (batch-minted, unattested), while other
similarly-sized owners clear **50–95%** attestation. Attestation is not
independent of ownership — which owner an agent belongs to strongly
predicts whether it clears rung 7 — but it is not explained by a small
number of owners dominating the whole population either. Both things are
true at once, and they are different claims from Q3's author-side
concentration (an owner having a 95% attestation rate says nothing about
whether that owner's agents were all vouched for by the same handful of
*author* addresses — which, per Q3, is common).

## 5. Correlation with document quality

```sql
with r7 as (
  select agent_id, status as r7_status from check_results
  where run_id = 'cfbfcc01-fdaf-409f-9bed-abf706d865c7' and rung = 7
),
r2 as (
  select agent_id, status from check_results
  where run_id = 'cfbfcc01-fdaf-409f-9bed-abf706d865c7' and rung = 2
)
select r7.r7_status, r2.status, count(*),
  round(100.0 * count(*) / sum(count(*)) over (partition by r7.r7_status), 1) as pct
from r7 join r2 on r2.agent_id = r7.agent_id
group by r7.r7_status, r2.status
order by r7.r7_status, r2.status;
```

| rung-7 (attested) | rung-2 (resolvable) | count | % within that r7 group |
|---|---|---|---|
| fail | pass | 17,875 | 58.6% |
| fail | fail | 11,117 | 36.4% |
| fail | error | 1,535 | 5.0% |
| **pass** | **pass** | **13,832** | **46.8%** |
| pass | fail | 14,247 | 48.2% |
| pass | error | 1,491 | 5.0% |

Attested agents are **not** more likely to have a resolvable document —
if anything, slightly *less* likely (46.8% resolvable vs. 58.6% for
non-attested agents). **53.2% of attested agents (15,738 of 29,570) have no
resolvable document at all** (rung-2 `fail` or `error`) yet still carry
Reputation Registry feedback. Attestation is essentially independent of
document quality, and mildly anti-correlated with it — an agent can fail to
resolve, parse, or conform to the spec at all and still pick up feedback
from the registry.

## Verdict

**29,570 (49.2%) is a real, reproducible count of agents with at least one
Reputation Registry feedback entry — that part of the finding stands.** But
"49.2% have feedback" is not the same claim as "49.2% received independent
attention from the ecosystem," and the evidence above does not support the
latter:

- The stored `feedback_count`/`distinct_authors` distributions are
  heavily right-skewed (median 2 / median 1), not flat — consistent with a
  small number of active authors doing most of the work, not thousands of
  independent reviewers each leaving one review.
- The live sample makes this concrete: **one address accounts for an
  estimated 42–53% of the entire attested population**, and **just 6
  addresses account for an estimated 84–91%**, with **88% of sampled
  attested agents' feedback coming from those 6 addresses and no one else**.
- Attestation is not explained by ownership concentration (no small set of
  *owners* dominates), and it is not explained by document quality (attested
  agents are, if anything, slightly less likely to have a working document).
  The one dimension where extreme concentration shows up is the *author*
  side — exactly the dimension stored evidence cannot see and this report's
  live sample was built to check.

**This is an artefact of concentrated activity, not genuine broad,
independent adoption.** Six client addresses, not 29,570 independent
reviewers, are responsible for the overwhelming majority of what this rung
counts as "attested." Publishing 49.2% without this context — e.g.
alongside the ~6% figure from prior academic work on participation — would
imply an ~8× adoption gap that the underlying activity does not support:
the prior work's ~6% may be closer to counting independent participants,
while this rung, by design (see `rung7_attested.rs`'s module doc), counts
*agents reached*, which a handful of addresses can inflate on their own.
This is a measurement-scope difference, not evidence that either number is
wrong on its own terms — but the two are not comparable without saying so.
