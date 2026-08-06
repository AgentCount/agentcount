# Methodology changelog

This document records every change to what AgentCount measures or how it
measures it — as distinct from ordinary code changes that don't touch check
semantics. Each entry is dated, states what changed and why, and reports the
measured population effect: how many agents' results moved, counted against
an archived run rather than re-swept from the chain, wherever that's
possible. `METHODOLOGY.md` describes the method as it stands today; this
file is the history of how it got there, including the parts where an
earlier version of the method was wrong.

Format per entry:

- **Date**
- **What changed** — the rule, in the same terms the code uses.
- **Why** — the evidence that motivated the change (spec citation, external
  review, internal audit).
- **Measured effect** — the population count this fix touches, and how it
  was obtained (fresh sweep vs. re-judging an archived run).

---

## 2026-08-06 — NEW MEASUREMENT: payments become a pinned, run-scoped pipeline, and the old 358 / 313 / 190 are superseded

**Read the last paragraph of this entry first if you are checking whether a
number moved. None did. This entry restates no previously published figure,
because there is no previously published figure to restate.**

**What changed.** A new measurement ships — not a rung, not a status, not an
evidence shape, and not a change to anything the seven rungs answer:

1. **A `payments` table** (migration 0019), plus `payment_targets` and
   `payment_scans`, all keyed by `run_id` exactly like `check_results`. Every
   read is at the run's `pinned_block`.
2. **An attribution rule, stated once and enforced in one place.** An incoming
   token transfer may be attributed to agent *A* only if it was credited to
   `getAgentWallet(A)` at the run's pinned block, that address is non-zero, and
   it is **not equal to** `ownerOf(A)` at the same block. The rule and the
   argument for it live in `crates/payments/src/lib.rs` and in
   `METHODOLOGY.md` §8.
3. **A binary**, `crates/sweeper/src/bin/payments.rs`, that runs after a sweep
   for a finished run, reads token transfer logs at that run's pinned block,
   and writes the rows. It is scheduled where `liveness` is scheduled, after
   the delta, and its failure is tolerated per chain in the same way.
4. **Four exclusions, each a named rule with a regression test** — `pre_mint`,
   `owner_funding`, `burn_address` and the three structural ones (`outgoing`,
   `self_transfer`, `mint_block_unknown`). Excluded rows are **stored** with
   the rule that excluded them, so the uncorrected figure stays recomputable
   and each correction is visible rather than asserted.
5. **A pure crate**, `crates/payments`, holding all of the above as functions
   with no database and no network, so `cargo test -p payments` exercises the
   attribution rule and every exclusion in CI with nothing configured.

**Why.** Because the numbers this project already had were produced by a
one-off log study and could not be recomputed by anybody, including us.

`analysis/payments-design.md`, `analysis/payments-per-chain.md`,
`analysis/payments-corrections-ledger.md` and
`analysis/x402scan-crosscheck.md` are a genuine piece of work: they established
the method, cross-checked it against an independent index, and caught four of
their own errors before publication. What they are not is a measurement of
record. Their figures are not in the database, not pinned to a block, and not
reproducible by a sweep — which is the exact property this census claims for
everything it publishes. §5 of `METHODOLOGY.md` says a result that cannot name
what generated it is an assertion rather than a fact. That standard has to
apply to this project's own most quotable numbers or it applies to nothing.

The rule in point 2 is the crux, and it is inherited from the corrections
ledger rather than re-derived. All four retractions there are **one mistake
wearing four costumes: an address was treated as an identity.**

- **PAY-1** — a payment to a shared address was credited to *every* agent
  declaring it. 298 addresses received an external transfer against 313
  declaring agents.
- **PAY-2** — transfers were counted over a wallet's entire history, including
  before the agent existed. 6,748 of 18,328 Base transfers, and **82.5% of all
  value**, predate the mint of the agent they were credited to.
- **PAY-3** — `owner()` of the receiving contract was never read and the
  contracts were never inspected, so Morpho vault flow was read as revenue.
  The "one operator" turned out to be 148 per-agent contracts controlled by
  **126 distinct addresses, none of them the registrant**.
- **PAY-4** — mainnet agent 28283 declares the zero address as its wallet, and
  the scan collected **313,255 of mainnet's 314,735 transfers (99.5%)** — every
  USDC and USDT burn on Ethereum — as that agent's income.

Every one of those is either impossible or explicitly recorded under the new
rule. PAY-4 in particular **cannot occur on the verified basis at all**:
nothing can produce an EIP-712 signature for the zero address, so
`setAgentWallet` cannot name one.

**On the choice of address, because it is the whole argument.** The alternative
was the `services[].name == "agentWallet"` convention. It is not in the spec, it
carries no proof of control, it is served over mutable HTTP, it survives a
transfer of the NFT that the registry's own value would have cleared, and on
Base it contradicts the registry's verified value for **409 of 919** agents
with a further **50** the registry has never verified. The verified basis is
also narrower for a reason that is part of the rule rather than a filter: of
the 40,473 Base agents with `getAgentWallet` set, **40,126 (99.1%) have it set
to the owner's own address** — the contract default, requiring no signature
from anybody, and not per-agent, since one owner holds 2,293 agents. Only
**347** verified a distinct address. Those 40,126 are recorded as
`wallet_equals_owner`, which is a different fact from "received nothing", and
the table keeps them apart.

The declared basis is **still computed on every run**, stored under
`basis = 'declared_wallet'`, and never published. The gap between the two is
the most honest thing this measurement produces, and refusing to compute it
would be a decision about what a reader gets to see.

**Measured effect — none, and this is the unusual part.**

*On any published figure: none.* No rung's rule changed, no agent's status
moves, no rate is recomputed, and no archived run is rewritten. `payments` is a
new table that every existing run has zero rows in, and a run with no rows in
it has **not been measured**, which is what absence has always meant here.

*On the previously circulated figures: they are superseded and unpublishable.*
Stated plainly, because three different numbers have been in circulation
internally and all three are now retired:

| figure | source | status |
|---|---|---|
| **358** agents paid across four chains, **34** via x402 | `analysis/payments-per-chain.md` §7, declared basis | **superseded** — not pinned, not recomputable |
| **313** agents paid on Base | `analysis/payments-design.md` §6 | **retracted** (PAY-1, PAY-2) and superseded |
| **190** agents paid on Base, post-mint | `analysis/identity-role-audit.md` §3 | **superseded** — correct under its own method, but declared-basis and not recomputable |

**This is a new measurement, not a restatement.** No figure above is being
re-derived, adjusted, or carried forward under a new label. The first number
this pipeline produces will be the first number it has ever published, and it
will come from a run rather than from an analysis document. Until such a run
exists, AgentCount publishes **no** payment figure, and the absence means what
an absent rung means: not measured.

*What a future run will make visible, which the old figures could not.* Because
both bases are stored side by side with a `basis` column, one query will give
the count on the address the spec verifies and the count on the address anyone
can write. The old study only ever attributed through the second
(`analysis/payments-per-chain.md` §1 explains why: the verified set could not be
confirmed against the live getter for every chain). Its single verified-basis
figure — **37 of Base's 347 distinct wallets** received an external transfer —
was measured **before** the pre-mint and contract-sender corrections and was
never recomputed with them, so it is a ceiling and not a result. Nothing here
predicts what the new number will be, and nothing should.

**Not done, deliberately.** The pipeline has not been run against production.
The deliverable is the rule and the rows; the figure comes from a run a
maintainer schedules, and inventing one in advance would be the same error as
publishing an analysis document's number as a census.
## 2026-08-06 — `refused`: a rate limit of ours was being published as 19,983 agents going dark

**This entry changes published numbers.** Every archived run is re-judged. If
you have quoted a rung-2 `fail` or `error` rate, or a `stopped_resolving`
figure, from any run before today, read the tables at the bottom: the numbers
moved, nothing was re-probed, and no agent gained or lost a `pass`.

**What changed.**

1. **A seventh status, `refused`, on rungs 2 (`resolvable`) and 6 (`live`):
   the origin is demonstrably there and declined this request.** Five HTTP
   statuses qualify, in the two groups the HTTP specification itself separates —
   **429 and 503**, the statuses defined to carry `Retry-After` and to mean "not
   now" (RFC 6585 §4, RFC 9110 §15.6.4), and **401, 402 and 407**, the statuses
   that answer with a challenge rather than an absence. They were rung 2's
   `fail`, whose published meaning is "their document is unreachable".
2. **A `robots.txt` we could not get permission from is `refused`, not
   `error`.** `robots_disallowed` and `robots_unavailable: …` were recorded as
   this checker having malfunctioned. It had not: we asked for permission, did
   not get it, and — deliberately, per RFC 9309 §2.3.1.4 — sent no request. The
   behaviour on the wire is unchanged. Only the word is.
3. **Rung 6 gets the same rule, with one deliberate exception.** 402 remains
   `pass` there ("a dead host does not bill you", the 2026-08-01 ruling); a 429
   does not, because a 429 is a statement about *our* request rate, and reading
   it as liveness would mean the harder we probe a host, the more live its
   agents appear. Precedence for a multi-endpoint agent is pass > fail >
   refused > error.
4. **Transitions into or out of `refused` are excluded from
   `stopped_resolving` and `newly_resolving`,** by rule, in
   `crates/sweeper/src/delta.rs`. They remain in `flips` — deleting the
   evidence would make the rate limit invisible, which is the same failure in
   the other direction.
5. **Politeness, so this recurs less:** a host now sees at most **one request
   started every 250ms** (on top of the existing cap of 2 in flight), and
   `Retry-After` on a 429/503 pushes that host's next request forward, capped
   at 120 seconds. See `METHODOLOGY.md` §6.
6. **Schema version 8**, checker `0.7.0`. Rung 2's evidence gains
   `retry_after`; rung 6's gains `endpoints_refused`.

**Why.** The 2026-08 census delta reported that **19,983 BSC agents had stopped
resolving**. That is the number this project exists to produce — registrations
are published by everyone, decay by nobody — and it was wrong.

19,962 of those 19,983 were HTTP **429**. 19,658 of them came from a single
host, `metadata.evoevo.ai`. Excluding 429 and 503, BSC lost **10** agents.

We generated those 429s. The prober capped concurrency per host at 2 but never
capped the *rate*, so a host answering in 20ms was being asked about 100 times a
second — and the four hosts that carry 59.2% of every declared endpoint in the
census are exactly the hosts asked most. The checker then booked each 429 as
rung 2's `fail`, the agent's word, so an infrastructure problem of ours became
19,983 separate accusations; and no error rate could see it, because `error` is
the word for our failures.

The same defect had a second face. `robots_unavailable: connection failed
fetching robots.txt` was `error`, and on the 2026-08 mainnet run **6,133 agents
sat behind one host** (`agents.exquisite.land`) whose `/robots.txt` refused
connections. That single host took mainnet's published error rate from ~10% to
**22.1%** — a number about that host's robots endpoint, presented as a number
about this checker.

Three words were available for both cases and all three were wrong. `fail`
blames the agent for something the agent did not do. `error` claims a
malfunction that did not happen. `pass` claims a document we never received.
`refused` is the fourth thing that actually occurred, and it is the same claim
in both cases: **the origin declined us, and we learned nothing about the
document.**

**The robots.txt policy, chosen explicitly rather than inherited.** Three
options existed for an unavailable `robots.txt`: treat it as permission granted
(the common crawler convention, and the one that would make our error rate look
best), treat it as denied (RFC 9309 §2.3.1.4's conservative reading), or make it
its own non-error observation. We took the conservative behaviour and the
honest record: **we still do not fetch**, and the outcome is now `refused`. A
robots.txt is the only channel a site operator has for saying no without knowing
we exist, and reading its failure as a yes takes the benefit of the doubt in our
own favour, on hosts that are by definition having trouble serving requests. The
cost — those agents' documents go unread — is now stated per row instead of
being absorbed into a number about ourselves.

**What did NOT change.** No rung's `pass` rule. No agent gained or lost a
`pass`, in either direction. Skip-propagation is untouched: `refused` is not
`pass`, so it stops a dependent rung exactly as the `fail` or `error` it
replaced did. 403, 500, 502 and 504 are still `fail` — a 403 refuses without
offering a way in, and a broken upstream means the document really is not being
served. The 2026-07-28 ruling that HTTP 402 must not be read as "alive" at rung
2 stands; a 402 still does not pass, it is simply now described as the challenge
it is, and its evidence keeps the distinct reason `payment_required`.

**Measured effect — rung 2, by re-judging the four archived 2026-07-29 runs.**
No chain was read and no request was sent: every row was re-judged from the
`http_status` and `reason` it already carried.

| run (2026-07-29) | pass | fail | error | refused | of which from `fail` | from `error` |
|---|---|---|---|---|---|---|
| bsc `f78c7891` (244,208) | 180,825 → 180,825 | 62,515 → 13,873 | 868 → 131 | 0 → 49,379 | 48,642 | 737 |
| base `cfbfcc01` (60,097) | 31,707 → 31,707 | 25,364 → 22,836 | 3,026 → 695 | 0 → 4,859 | 2,528 | 2,331 |
| mainnet `18a25593` (40,806) | 17,622 → 17,622 | 19,012 → 18,076 | 4,172 → 197 | 0 → 4,911 | 936 | 3,975 |
| celo `7833fc49` (9,747) | 9,494 → 9,494 | 143 → 141 | 110 → 61 | 0 → 51 | 2 | 49 |

Rung 2's **error rate** falls accordingly: mainnet 10.22% → 0.48%, base 5.04% →
1.16%, celo 1.13% → 0.63%, bsc 0.36% → 0.05%. The **fail** rate falls furthest
on BSC, 25.6% → 5.7%, which is the 48,635 agents that were only ever rate
limited. What is left in `error` is what the word always claimed: our timeouts,
our TLS failures, our IPFS gateways.

The largest single contributors, so the concentration is on the record rather
than implied: `metadata.evoevo.ai` (47,863 rung-2 429s on BSC),
`gateway.pinata.cloud` (2,413 on Base), `api.normies.art` (934 on mainnet),
`agents.exquisite.land` (3,859 robots-unavailable on mainnet — 6,133 by the
2026-08 run), `wild-west-bots.vercel.app` (1,582 robots-disallowed on Base).

**Measured effect — rung 6, same four runs.** Re-judged by re-running the
`liveness` pass, which reads the archived `endpoint_probes` rows and sends no
new requests:

| run | fail | error | refused |
|---|---|---|---|
| bsc | 4,233 → 4,233 | 33 → 1 | 0 → 32 |
| base | 2,385 → 2,348 | 332 → 3 | 0 → 366 |
| mainnet | 268 → 137 | 144 → 9 | 0 → 266 |
| celo | 17 → 17 | 113 → 1 | 0 → 112 |

`pass`, `skipped` and `unprobeable` are unchanged on every chain, at every rung.

**Measured effect — the delta that started this.** The 2026-08 BSC delta
reported `stopped_resolving: 19,983`. With both runs re-judged, the 19,962
rate-limited agents are `refused` on the newer side and excluded by rule, and
the same delta reports **10** — the agents that actually stopped answering. The
19,962 stay visible in `flips` as `pass → refused`, which is where the finding
came from in the first place.

**The 2026-08 runs' own before/after table is produced by the backfill, not
typed here.** Those archives are not local to this change, and a count written
into this file from memory is the exact failure mode it exists to prevent. Run

```
DATABASE_URL=… cargo run -p sweeper --bin backfill-refused          # dry run
DATABASE_URL=… cargo run -p sweeper --bin backfill-refused -- --apply
DATABASE_URL=… cargo run -p sweeper --bin liveness <chain> <run-id> # per run
```

The first prints the table above for every run in the database, 2026-08's
included, without writing anything.

**Published archives are not reissued.** `data.agentcount.ai` holds immutable
bytes, which is the whole point of publishing at a permanent URL. An archive
stamped schema ≤ 7 carries the old words, and `DATA.md` gives the mechanical
mapping to schema 8.

---

## 2026-08-04 — NOT A CHECK-SEMANTICS CHANGE: three chains produced no census at all, and minter capture now works where it never had

**What changed.** Two things, neither of which touches a rung's rule, a status,
an evidence shape or the schema version:

1. **A response that cannot be decoded is now treated as permanent.** The RPC
   retry wrapper in `crates/chain` retried anything whose error message
   contained `429`, `rate limit`, `too many requests` or `compute units per
   second`. A decode failure is now classified first, and never retried:
   the bytes on the wire will not be different next time.
2. **The minter — the sender of an agent's registration transaction — is read
   out of a raw `eth_getTransactionByHash` response's `from` field**, instead of
   by decoding the whole transaction into a typed value. It was never the
   transaction the census wanted, only the sender.

A third change is operational rather than methodological, and is recorded here
because it is why the incident below was misread for as long as it was:
`scripts/weekly-sweep.sh` reported `sweep exited 0` for runs that had exited 75.
It read `$?` inside an `if ! sweeper …; then` branch, where `$?` is the status
of the inverted test, not of the sweeper. The log now names the stall and the
real exit code.

**Why.** On 2026-08-04 a four-chain sweep produced **one** census. `mainnet`
finished with 46,987 agents. `base`, `celo` and `bsc` were each killed by the
stall watchdog after 900 seconds having written **zero** agents.

The cause is that agent ids and their registration transactions are read
through different code paths, and only the second one decodes a transaction.
alloy's transaction-type enum accepts `0x0`–`0x4`. Celo's CIP-64 fee-currency
transaction is type `0x7b` (123) and an OP-stack deposit is `0x7e` (126), so
every attempt to read a registration transaction's sender on those chains
failed with ``unknown variant `0x7b` ``. mainnet, which mints only standard
types, was the one chain with nothing to trip over — the single-chain success
was the clue, not the consolation.

That alone would have cost the minter and nothing else, because minter capture
is explicitly allowed to fail. What turned it into three lost censuses is that
the failures were retried. alloy formats a decode failure as
`deserialization error: {err}\n{text}` — with the ENTIRE raw response body
inside the error message — and the throttle check was a substring search over
that message. A transaction whose calldata, hashes or ECDSA signature happen to
contain the characters `429` was therefore read as "the provider is rate
limiting us" and re-requested eight times with exponential backoff, ~76 seconds
per transaction, for bytes that could not change. Sampled against live Celo
blocks the same day, **6 of 138 (4.3%)** non-standard-type transaction
responses collide with `429` that way; the median response body is ~1,050
characters, nearly all of it hex. At the shipped `RPC_CONCURRENCY` of 3, a few
hundred colliding transactions is the whole 900-second window.

Minter capture runs as a pre-pass, before the first agent is written, so all of
that time was spent by a run that had produced nothing. It now also has a wall
clock budget (two thirds of the stall timeout, `MINTER_CAPTURE_BUDGET_SECS`),
keeping whatever it resolved and letting the sweep proceed. Provenance must not
be able to outrank the census.

**Measured effect — read this as two separate claims.**

*On any published figure: none.* No rung's rule changed, no agent's status
moves, no rate is recomputed, and no archived run is affected. All four
published runs are `schema_version` 5, which predates minter capture entirely,
so there is no published minter data for this to change. Re-judging an archived
run was not applicable and was not done.

*On what the next sweep records: real, and in one column.* The `minter` field
of `agent_snapshots` (added in schema 6, migration 0013) would have been null
for every agent on `celo`, `base` and `bsc` had those runs completed, because
every sender read on those chains failed. Reading `from` directly restores the
field on all three. The honest bound on the size of that is: **every** agent
whose registration transaction is a non-standard type gets a minter it would
not otherwise have had, and the incident's own logs show that population was
100% of attempted reads on the affected chains. The exact per-chain coverage is
not asserted here because it has not been measured against a completed run —
the first sweep to finish under this fix will report it as
`minters resolved for N/M transactions`, and that line is the number to quote.

The schema version is deliberately NOT bumped. `minter: None` already means
"this run did not capture it" and never "this agent has no minter", which is
exactly what an unresolved sender still means; and since no run at schema ≥ 6
has been published, there is no archived row whose reading this could make
ambiguous.

**What was verified, and what was not.** The transaction type, the exact serde
error text, and the `429` collision rate were all measured against live Celo
data on 2026-08-04, and the regression tests in `crates/chain/src/registry.rs`
carry a real CIP-64 transaction — type `0x7b`, with `429` inside its signature
— as their fixture. What has not been done is a full sweep of the three
affected chains; until one finishes, "minter capture works on celo, base and
bsc" is a claim resting on the RPC responses those chains actually return, not
on a completed census.

---

## 2026-08-03 — NOT A CHECK-SEMANTICS CHANGE: a continuous registration tail, kept outside the census

**What changed.** No rung's rule, no evidence shape, no status, no schema
version. Nothing about how any agent is judged, and nothing about how any
published figure is computed. This entry exists because a new source of agent
data now appears on the site, and a reader is entitled to know exactly what it
is and what it is not.

Between censuses, a poller (`sweeper`'s `tail` binary) reads each enabled
chain's current highest agent id — the same contiguous-id binary search over
`ownerOf` the census uses for enumeration — and records, for every id above the
last sweep's, only what an on-chain read gives cheaply: the owner, the declared
URI, and the block both were read at. Those rows make a newly registered agent
findable and linkable. They carry **no check results of any kind**, and no
census figure includes them.

**Why.** An agent minted after a sweep was invisible: search returned nothing
and its permalink 404'd. The person most likely to hit that is the registrant,
minutes after minting, and the reasonable conclusion from a 404 is that the
site is broken. Discovery is the cheap half of a census (~17 RPC calls to find
the top of the id range, two more per new agent); the expensive half — fetching
each document, probing endpoints, judging seven rungs — is what makes a sweep a
scheduled multi-hour job. Doing the cheap half continuously closes the gap
without touching the pin.

**How the boundary is enforced.** Structurally, not by a filter anyone has to
remember. `registration_tail` (migration 0018) has no `run_id` and no foreign
key to `runs`. Every census aggregate — every rate, finding, delta and export —
is computed by joining down from a `runs` row, so no such query can reach a
tail row at all. The alternative considered and rejected was a flag column on
`agent_snapshots`: that inverts the default, because every existing aggregate
would silently start including unchecked agents until someone added
`WHERE is_tail = false` to all of them, and the one that got missed would be
the one that published a wrong number.

Three further consequences, all deliberate:

- The API returns a tail agent in a shape with **no rungs array at all** — not
  an empty one — marked `"source": "tail"`, sharing no field name with a census
  result beyond `chain` and `agent_id`. A client that ignores the discriminator
  cannot render it as a checked agent; it breaks visibly instead. An empty
  array was rejected as the more dangerous shape: "seven statuses missing" and
  "seven statuses failed" look alike in a UI.
- Tail results never merge into a run-scoped list. `/api/agents` carries them
  in a sibling `tail` array beside `items`; `/api/tail` serves them on their
  own. `total` and every count on that page remain the run's.
- Once a census run covers an id, its tail row is marked superseded and stops
  being served — the agent's real answers come from the run, and the row
  survives only as the record of when the id was first seen.

**Measured effect. None — no agent's result moves.** Not one `check_results`
row is written, read, altered or re-judged by any of this; no run's
`agent_count` changes; no archive gains or loses a byte; the 354,858 population
figure and every rate derived from it are untouched. The effect is on what can
be *found* between sweeps, not on what has been *measured*: an agent minted
after the last pin now resolves at its permalink as an explicitly unchecked
discovery instead of returning 404. The count of such agents is published per
chain at `/api/tail/summary`, as a count of things this census has not
measured.
## 2026-08-03 — NOT A CHECK-SEMANTICS CHANGE: on-demand spot checks ship, and no published number moves

**What changed.** A new endpoint, `POST /api/agents/{chain}/{id}/spot-check`,
and a button on every agent page: run the conformance ladder for that one
agent against the chain's current head and show the answer now, instead of
waiting for the next sweep. `METHODOLOGY.md` gains a Section 5 subsection
("Spot checks are not measurements of record") and two Section 6 bullets.

**No rung's rule changes.** Not one line of `crates/checks` was touched, and
this entry exists to say so rather than to record a semantic change. The
endpoint calls the same pure checker functions the sweeper calls
(`registered`, `resolvable`, `parseable`, `conformant`, `bound`, `attested`)
and takes its skip-propagation from `checks::run_ladder`, the same single
implementation the sweeper and `/api/validate` use. A spot check and a census
row can disagree about the world — a server that was up in July may be down
today, which is the point of the feature — but they cannot disagree about
what a status means, because there is only one implementation of the meaning.

The one code move in service of that: `crates/sweeper`'s private
`checks_scheme` — which reduces a fetch outcome to the scheme bucket rung 2
is judged against — became `probe::FetchOutcome::scheme_bucket`, so the
sweeper and the spot check share it rather than each keeping a copy free to
drift. Behaviour is byte-identical; the sweeper's own function is now a
one-line delegation.

**A spot check is not a run, and never enters a published figure.** It has no
`run_id`, because it belongs to no run. It is **not stored** — no table, no
migration, no row anywhere — so no census query can reach it by forgetting a
filter, and nobody can later aggregate a self-selected sample of "agents
somebody happened to click on" into something that looks like a rate. Its
response carries `"source": "spot_check"`, a notice in the body, and its own
`checked_at` / `block_number` / `checker_version` / `checker_commit` /
`schema_version` / `spec_commit`; it shares no top-level field name with the
census's agent-detail response, so a screenshot of one cannot be read as the
other.

**Rung 6 (`live`) is not probed on demand**, and its absence carries the
existing not-checked meaning — no row, never a guessed status — with the
reason stated explicitly in the response. Rung 6 sends one request per
*declared* endpoint, a document may declare any number of them at any hosts
its author chose, and there is no run to apply the census's
500-distinct-URLs-per-host budget against. An on-demand rung 6 would
therefore be an unbounded burst aimed at a target the caller picks. Rungs 1
to 5 and 7 are asked; rung 6 answers come from runs or not at all.

**Probing etiquette (Section 6) gains two commitments**, because this is the
first thing here that lets a stranger cause us to send a request. Five spot
checks per ten minutes per caller, and — the limit that actually protects
third parties, since it is keyed on the host of the URI the agent published
on chain rather than on anything the caller controls — one per minute and
twenty per hour per target host. Rotating addresses or agent ids does not
move the second number. Over the limit is a `429` with `Retry-After`.
Everything else is the census's existing discipline unchanged: the same
`probe::Prober`, so the same identifying User-Agent and contact mailbox, the
same robots.txt handling including its redirects, the same per-host
concurrency cap, the same timeouts, redirect cap and response-size cap, and
the same SSRF netguard re-validated on every hop. Only agent ids the Identity
Registry confirms exist can be checked; there is no arbitrary-URL probing
endpoint.

**Measured effect. None.** No rung's rule changed, no archived result was
re-judged, no published rate, finding, archive or count is affected, and no
agent's status moves. `published-runs.json`, `docs/reports/` and `analysis/`
are untouched by this change.

---

## 2026-08-02 — CLAIM CORRECTION: the homepage said "the four largest chains", and it was false

**What changed.** No rung's rule. The site's H1 claimed the census covered
"the four largest chains" by ERC-8004 registrations. Nobody had verified the
ranking before publishing it. Counting registrations on every chain the
canonical registry is deployed on — by binary search on `ownerOf` at
`0x8004a169…a432`, the census's own population definition, validated by
reproducing the census's Celo count exactly at its pin time — places the
swept chains at **#1 (BNB Chain), #2 (Base), #3 (Ethereum mainnet), and #8
(Celo)**. Billions (chain id 45056) held 25,974 agents at the census's own
pin date, 2.7 times Celo's population; MegaETH, X Layer and Monad also
exceed Celo. The claim was false at publication, not stale.

**Why.** The 2026-08 product review checked it. The claim originated when
Celo was added for other reasons and the H1 was written as if population
rank had been the selection criterion.

**The fix.** The H1 now names the chains it counted, derived from the
published-runs list at render time, and links to a new `/coverage` page
listing every known deployment with its count — probe script committed, one
command to recompute. No coverage percentage or chain count is typed
anywhere in the site's copy. The footer and methodology sentences that said
"every AI agent registered under ERC-8004" unqualified are scoped the same
way. As of 2026-08-01 the swept chains hold 83.0% of the 439,582
registrations the probe could count; that figure lives on `/coverage`,
computed from the committed probe data, and is not quoted in the H1 because
it moves.

**Measured effect.** No agent's result moves — this was a claim about scope,
not a measurement. The 354,858 population figure was and remains correct.

---

## 2026-08-02 — PROVENANCE CORRECTION: the published index misstated which checker swept the four canonical runs (no semantic effect)

**What changed.** No rung's rule, and no archive byte. `published-runs.json`
recorded all four canonical runs as `schema_version: 7`,
`checker_version: 0.6.0`. The database — served verbatim by `/api/runs` —
records all four as `schema_version: 5`, `checker_version: 0.5.0`, and the
homepage displayed those values beside a `/data` page displaying the others:
the same run, two different checkers, on one site. The checker *commits*
were correct everywhere; only the version and schema numbers were wrong, and
they belonged to the era of the 2026-08-01 rebuild that produced the
archives, not to the sweeps.

**Why.** The four runs were swept before anything published exports, so
their archives were rebuilt from the database on 2026-08-01, and the rebuilt
manifests carried the rebuild era's schema and checker versions in the
fields that should have named the sweep's. The exact path by which the wrong
values reached the manifests was not fully reconstructed; the effect is
plain in the artifacts.

**The fix.** The index now records the sweep-time values (`5` / `0.5.0`,
matching the database) plus each archive's `rebuilt_at`. Manifests gain
`exporter_version` / `exporter_commit` so the writer and the judge are never
again one field; `publish-run.sh` carries both into the index. The archives
themselves are immutable and keep their overstated internal manifests —
where an archive manifest and the database disagree, the database is
authoritative, and `METHODOLOGY.md` §5 now says so.

**Also adopted.** Canonical runs are swept from clean commits only, from the
next sweep onward. Three of the four July runs carry a `-dirty` checker
commit — an honest stamp of an unreproducible build. They stay published;
the policy prevents a fifth.

**Measured effect.** None on any agent or rate. Four index entries
corrected; the archives' sha256 hashes are unchanged and re-verified against
the published checksums during this correction.

---

## 2026-08-05 — `checker_commit` was `unknown` for the whole 2026-08 census

**What changed.** The git SHA is now passed into the weekly job's image build
explicitly, instead of being read by `git rev-parse` at build time.

**Why.** `crates/sweeper/build.rs` stamps the commit by shelling out to git.
That works on a workstation and cannot work in the deployed job: `.dockerignore`
excludes `.git` (correctly — a repository in an image layer is bulk nobody
needs) and Cloud Build uploads the source as a tarball regardless. The command
failed, the script fell back to its `"unknown"` placeholder, and every run of
the first scheduled census recorded a provenance field that names nothing.

The fallback did its job — it did not invent a plausible SHA — but "unknown" is
not usable provenance, and `checker_commit` is specifically the field that lets
a reader fetch the code that produced a number. This project's central claim is
that its results can be recomputed; a run that cannot say what produced it is
the one case where that claim is not checkable.

**Measured effect.** No agent's status moves and no rung's rule changes. Five
published runs carry `checker_commit: unknown` permanently:

| chain | run | agents |
|---|---|---:|
| bsc | `e6f87cdb` | 251,782 |
| base | `24959257` | 60,589 |
| mainnet | `acea7d5d` | 47,001 |
| mainnet | `4514e591` | 46,987 |
| celo | `974f0c12` | 9,758 |

They are **not being reissued**. The archives are immutable, which is what
makes a committed hash worth anything, and replacing bytes to correct a
metadata field would break that guarantee to fix something a sentence can fix.
The code that produced them is commit `3235667` or an immediate ancestor,
checker `0.6.0`, schema `7`; `checker_version` and `spec_commit` are correct on
those runs and unaffected. `DATA.md` says the same where a reader of the data
will meet it.

---

## 2026-08-01 — Rung 6 (`live`) ships, with a sixth status and a disclosed sample

**What changed.** The service track stops being absent from every result. A
new pure checker, `crates/checks/src/rung6_live.rs`, answers "does anything
answer at the endpoints this document declares", and a new binary
(`crates/sweeper`'s `liveness`) collects the observations it judges. Schema
version 6 → 7; checker 0.5.0 → 0.6.0; migration 0015.

Four decisions — three settled in advance, one made here:

1. **Live means 2xx *or* 402.** A payment challenge proves something is
   listening; a dead host does not bill you. Counted separately as
   `endpoints_payment_gated`. This is the **opposite** of rung 2's ruling on
   the same status, and both are right: rung 2 asks whether the document
   could be retrieved (a 402 means no), rung 6 asks whether anything is alive
   (a 402 means yes). Anyone reconciling a 402 count across the two rungs
   needs that sentence, so it appears in `METHODOLOGY.md` §2 under both.
2. **Only `http(s)` is probed, and having nothing probeable is its own
   status.** `check_results.status` gains `unprobeable`, produced only by
   rung 6, for an agent whose every declared entry is a CAIP-10 chain
   address, an email address, an empty string, an `ipfs://` URI, or carries
   no `endpoint` field. That is `unclaimed`'s reasoning one rung over, and
   deliberately a *different* word: an agent that published a CAIP-10 address
   made a claim, it is simply not one you can dial.
3. **Distinct URLs are probed once, and mega-hosts are sampled** at 500
   distinct URLs per host, chosen by a fixed FNV-1a hash of the URL so the
   sample is identical on a resume, a re-run, or someone else's machine. An
   agent whose every URL fell outside the budget gets **no rung-6 row** — it
   is not assigned its host's rate, because that would be inventing a status
   for an agent nobody checked.
4. **Aggregation was not among the settled rulings, and is decided here:** an
   agent passes if **any** probeable endpoint answered live. See below.

**Why.** The rung was fully specified and unimplemented since the ladder was
written, blocked on two things, both now cleared. The method needed settling
(402 is payment-gated rather than dead; a `GET` 404 may front a POST-only
service; liveness is not functionality). And the probe's User-Agent had to
carry a domain that resolves and a mailbox that answers before it was decent
to send a single request — `agentcount.ai` and `probes@agentcount.ai` were
confirmed working end-to-end on 2026-07-30.

**Three changes to the previously published specification.** That
specification was public, and this file exists so nothing is quietly
overwritten:

- **Pass was "every declared endpoint returns 2xx"; it is now "at least
  one".** Sampling forces it. Once some of an agent's endpoints may go
  unprobed, "every endpoint answered" is not a claim we can make about a
  partially-probed agent, and an all-must-pass rule would have had to return
  no row at all for every multi-endpoint agent it sampled. The any-match rule
  is what rung 5 already uses for `registrations`, and it matches the question
  the rung is named for — whether the agent is reachable, not whether every
  URL it lists is perfect. Every endpoint's own outcome is in evidence with
  the counts, so the all-must-pass definition stays computable from published
  rows by anyone who prefers it.
- **Zero declared endpoints was "a rung-4 concern, not this one"; it is now
  `unprobeable` here.** Rung 4 still checks `services` presence. Leaving rung
  6 silent about an agent it plainly cannot probe conflated "declared
  nothing" with "was not sampled".
- **The method was "HEAD, falling back to GET"; it is GET only.** Enough
  servers answer HEAD with a 405 or 404 while serving the same URL correctly
  on GET that the fallback would have produced false `fail`s — an accusation
  about a working service, traded for a bandwidth saving.

**Measured effect**, computed against the four archived runs (`cfbfcc01…`,
`f78c7891…`, `18a25593…`, `7833fc49…`) **before any request was sent**:

| | |
|---|---:|
| declared `http(s)` entries, rung-4-passing agents | 124,364 |
| distinct URLs after exact-string dedupe | **62,243** |
| distinct hosts | 3,348 |
| hosts over the 500-URL budget | **12** |
| distinct URLs actually requested | **14,494** |
| agents declaring at least one probeable endpoint | 56,794 |
| agents that receive a rung-6 row | **27,956 (49.2%)** |

The budget does nearly all the work, and it is worth being precise about why:
deduplication alone only halves the request count, because the largest hosts
give each agent its own path — `evoevo.ai` declares 26,147 distinct URLs for
26,273 agents. Raising the budget to 5,000 would take coverage from 49.2% to
62.6% at 2.4× the requests, aimed almost entirely at one server. 500 was kept.

**No existing agent's status moves.** No other rung's rule changes, and a row
written under schema ≤ 6 has no rung-6 sibling — that absence continues to
mean exactly what it always meant. The schema bump is how a reader tells,
from the row alone, whether a missing rung 6 means "this run did not
implement it" (≤ 6) or "this run implemented it and this agent was not
probed" (7).

---

## 2026-07-28 — FIX 1: accept `endpoints` as a legacy alias for `services`

**What changed.** Rung 4 (`conformant`) now reads the services array as
`services.or(endpoints)` instead of requiring the literal key `services`.
- Both fields present → `services` is used; evidence records
  `both_fields_present: true`.
- Only the legacy `endpoints` key present → it is used, the check **passes**,
  evidence records `legacy_endpoints_field: true`.
- Neither present → fails, exactly as before this fix.
- Evidence always carries `services_field_source`
  (`"services"` / `"endpoints"` / `"neither"`), so which name each agent
  used — and therefore the population's migration rate — is queryable
  from stored results without a new sweep.

No agent is ever failed solely for using the legacy field name.

**Why.** The first census (run `1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a`) failed
25,636 agents at rung 4. External review flagged that an unknown share of
those declare their endpoints under the legacy name `endpoints` and are
otherwise conformant. Verified against the pinned spec
(`spec/ERC8004SPEC.md`, commit `68fc676`): the schema block that establishes
the governing MUST names the field `services` (line 62), but the prose was
never fully updated — it says "endpoints" at lines 115, 117, 121, and 402
(and, in the non-normative commented-out Test Cases block, line 416). This
internal inconsistency corroborates the migration story: `services` was
introduced in a January-2026 spec update and the prose references were not
all updated to match. 8004scan's metadata profile documents the same
migration explicitly: both names are valid, `services` takes precedence
when both are present, and legacy-only use produces a warning (`WA031`),
never a failure. Full citation trail: `spec/REQUIRED_FIELDS.md`.

**Measured effect.** Queried directly against the archived response bodies
in `http_archive` for run `1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a` (60,049
agents; no re-sweep) — every document that (a) failed rung 4 in that run,
and (b) lacks `services` but has `endpoints`:

> **525 agents** fail rung 4 in the archived run solely because their
> registration document uses the legacy `endpoints` field name instead of
> `services`.

Of those 525, 287 have `services`/`endpoints` as their *only* missing
required field — under this fix alone they flip from `fail` to `pass`. The
remaining 238 also declare only `endpoints` but are additionally missing at
least one other unconditionally required field (chiefly `x402Support` /
`active`); they stay `fail` after this fix and are addressed, if at all, by
FIX 2. Both counts describe the same archived run and are informational —
they will change when the census reruns with all eight fixes applied and
are not, on their own, a claim about the post-rerun `conformant` rate.

Query bodies (`convert_from(body,'UTF8')::jsonb`) guarded per-row against
non-UTF8 / non-JSON bodies (2,046 documents fail rung 3 for exactly this
reason) so malformed bodies are counted separately rather than aborting the
query; zero such bodies appeared among the rung-4 failures queried here,
since a rung-4 failure implies rung 3 (`parseable`) already passed.

---

## 2026-07-28 — FIX 2: remove `x402Support` and `active` from required

**What changed.** Rung 4 (`conformant`) no longer checks `x402Support` or
`active`. `UNCONDITIONAL_FIELDS` drops from seven entries to five:
`type`, `name`, `description`, `image`, `services`. Neither field is
reported in `fields_missing` any more, and a document lacking one or both
no longer fails the rung on their account.

**Why.** The first census (run `1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a`)
failed 6,269 documents at rung 4 on `x402Support` alone. Verified against
the pinned spec (`spec/ERC8004SPEC.md`, commit `68fc676`): both fields
appear *only* inside the illustrative example JSON block (lines 99 and
100) and are never mentioned anywhere in the prose with a normative
keyword (no MUST, SHOULD, MAY, or "mandatory" sentence names either one).
The original extraction (`spec/REQUIRED_FIELDS.md`) had treated every key
of that example as REQUIRED under line 54's governing MUST, absent a more
specific downgrade — a strained reading in general, and especially so for
`x402Support`, where it means an agent is judged non-conformant for
failing to affirmatively declare that it does *not* support a payment
protocol it may have no reason to mention at all. 8004scan's metadata
profile classifies both fields MAY. Full citation trail and the reversed
ruling: `spec/REQUIRED_FIELDS.md` Ruling 3. FIX 3 will give both fields a
formal MAY classification; this fix only removes them from the required
set.

**Measured effect.** Re-judged directly against the archived response
bodies in `http_archive` for run `1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a`
(60,049 agents; no re-sweep). Of the 25,636 documents that failed rung 4
in that run:

> **6,573 agents** flip from `fail` to `pass` under **FIX 2 alone**
> (dropping `x402Support`/`active` from the required set, `services`
> still checked under its literal name only — no legacy alias).
>
> **6,932 agents** flip from `fail` to `pass` under **FIX 1 + FIX 2
> combined** (the legacy `endpoints` alias plus dropping
> `x402Support`/`active`). This is 359 more than FIX 2 alone: of the 525
> agents FIX 1 identified as declaring only the legacy `endpoints` field,
> 287 already flip under FIX 1 by itself; the other 238 stayed `fail`
> because they were also missing at least one other unconditionally
> required field — 72 of those 238 were missing only `x402Support`
> and/or `active` in addition to using the legacy field name, and it is
> exactly those 72 (plus the 287) — 359 in total — that additionally flip
> once both fixes apply together.

Combined with the 4,175 agents that already passed rung 4 in the archived
run, applying FIX 1 + FIX 2 together against that same archived evidence
yields:

> **11,107 / 60,049 conformant — 18.5%**, up from the archived run's 7.0%
> (4,175 / 60,049).

The work order's stated expectation was "`conformant` moves ~7.0%
(4,175) → ~17% (~10,400)." The measured combined figure, 18.5%
(11,107), is higher than that estimate by about 1.5 percentage points —
roughly 700 more agents passing than predicted. This is reported as
measured, not adjusted to match the estimate; the estimate undercounted,
most likely because it did not account for the 359-agent overlap between
the two fixes (agents whose only obstacles were the legacy field name
*and* the two now-dropped fields, who need both fixes together to flip).

Query methodology (guarding non-UTF8/non-JSON bodies, non-object
documents, and the conditional `registrations[].agentId`/`agentRegistry`
check, which is unaffected by this fix and reapplied unchanged) follows
the same approach established in the FIX 1 entry above; zero unparseable
bodies appeared among the 25,636 rung-4 failures queried, consistent with
a rung-4 failure implying rung 3 (`parseable`) already passed.

---

## 2026-07-29 — FIX 3: split rung 4 by RFC 2119 severity (MUST / SHOULD / MAY)

**What changed.** Rung 4 (`conformant`) no longer collapses every field
into one required/not-required list. It now classifies each field it looks
at into one of three RFC 2119 severities and reports all three, always:

- **MUST (2 fields, conditional):** `registrations[].agentId`,
  `registrations[].agentRegistry` — checked only when `registrations` is
  present as a non-empty array. **This is the only thing that can fail the
  rung.** `pass` = zero MUST violations; `fail` = one or more.
- **SHOULD (7 checks):** `type`, `name`, `description`, `image` (reverses
  the 2026-07-27 ruling that had these REQUIRED — see below);
  `services`/`endpoints` (empty array recorded distinctly from an absent
  key: `services_status` is `"absent"` / `"empty"` / `"present"`, and
  `should_gaps` carries `"services"` or `"services_empty"` accordingly);
  `registrations` itself, at least one entry; `services[].version`
  (aggregated to one gap regardless of how many entries lack it).
- **MAY (3 fields):** `x402Support`, `active`, `supportedTrust`. A fourth
  field the work order's MAY table named, `updatedAt`, is **not** checked —
  it does not appear anywhere in the pinned spec at the pinned commit; see
  "Spec discrepancy" below.

Evidence always carries `must_violations[]`, `should_gaps[]`, `may_gaps[]`
— on both `pass` and `fail`, never just the array relevant to the verdict.
`schema_version` bumps from 1 to 2 for this evidence-shape change.

**Why — reversing Ruling 1.** `spec/REQUIRED_FIELDS.md`'s Ruling 1
(2026-07-27) held that `type`/`name`/`description`/`image` stay REQUIRED
because line 115's SHOULD ("...SHOULD ensure compatibility with ERC-721
apps") was read as constraining their *content*, not their *presence*.
Revisited under FIX 3's requirement to classify every field by severity
rather than in isolation, that reading did not survive being checked
against Ruling 2's own reasoning, decided the same day: Ruling 2 read line
123's structurally identical "SHOULD have at least one registration... and
all fields... are mandatory" as downgrading *presence* while a more
specific clause governs content — the opposite assignment Ruling 1 made for
the same sentence shape, applied to different fields. Ruling 4 (new, dated
2026-07-29 — Ruling 1 is kept in the record, not deleted) applies the same
rule Ruling 2 already used. Full reasoning: `spec/REQUIRED_FIELDS.md`
Ruling 4.

**The finding, not softened.** Combined with Ruling 2 (already conditional)
and Ruling 3 (`x402Support`/`active` never MUST), the registration file has
exactly **one MUST, and it is conditional**. Almost every document that
parses as JSON at all now passes rung 4. This is expected and is reported
as measured below, not adjusted to look more discriminating.

**Spec discrepancy vs. the work order (ground rule 1).** The work order's
MAY table lists `x402Support`, `active`, `supportedTrust`, `updatedAt`.
`updatedAt` does not appear anywhere in `spec/ERC8004SPEC.md` at the pinned
commit (`grep -in updatedAt` returns nothing) — verified against both
sources ground rule 1 names: `eips.ethereum.org/EIPS/eip-8004` agrees the
pinned text has no such field; `best-practices.8004scan.io`'s metadata
profile *does* define `updatedAt` (an optional freshness-tracking
timestamp), which is evidently the work order's actual source for this
entry. Per ground rule 1 ("if they disagree with this document, the spec
wins and the disagreement is reported back, not silently resolved"),
`updatedAt` is not checked at any severity — checking it would mean
inventing a field the pinned spec does not define, which this rung's own
founding discipline ("nothing added, nothing inferred from outside
knowledge of ERC-8004") rules out. Full citation: `spec/REQUIRED_FIELDS.md`
§MAY.

**Measured effect.** Re-judged directly against the archived response
bodies in `http_archive` for run `1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a`
(60,049 agents; no re-sweep) — using the actual, current
`checks::conformant` function itself (not SQL logic re-derived from it), so
this measurement cannot silently drift from what the shipped code does.
Population: the 29,811 documents where rung 3 (`parseable`) already passed
— exactly the population rung 4 is ever asked about, taken unmodified from
the already-stored, un-touched-by-this-fix rung-1/2/3 verdicts. Zero of
those 29,811 archived bodies failed to re-parse (consistent with rung 3
having already validated them).

> **New rung-4 pass rate: 29,552 / 60,049 — 49.2%** (99.1% of the 29,811
> documents that reach rung 4 at all). Up from the originally published
> 4,175 / 60,049 (7.0%) — **+25,377 agents** — and from the FIX 1+2 combined
> figure of 11,107 / 60,049 (18.5%) — **+18,445 agents**. 259 documents
> (0.9% of those reaching rung 4) still fail: every one of them has a
> `registrations` array present with at least one entry missing `agentId`
> and/or `agentRegistry` — the only way to fail rung 4 under this fix.

**The SHOULD-completeness distribution** — the new headline measurement,
over the 29,811 documents that reach rung 4 (a document that never resolved
or never parsed has no SHOULD-gap information to report, the same
population-scoping rung 4's pass/fail already uses):

| SHOULD gaps | Documents | % of 29,811 |
|---:|---:|---:|
| 0 | 897 | 3.0% |
| 1 | 3,337 | 11.2% |
| 2 | 12,271 | 41.2% |
| 3 | 3,120 | 10.5% |
| 4 | 10,135 | 34.0% |
| 5 | 36 | 0.1% |
| 6 | 15 | 0.1% |

Only **897 documents (3.0%)** satisfy all seven SHOULD checks. The
distribution is bimodal, clustering at 2 and 4 gaps rather than spread
evenly — consistent with two largely-independent failure modes (most
documents either supply `registrations` or don't; most either supply
`services`+`type`+`image` as a bundle or omit all three) rather than one
smooth quality gradient. Nothing in this run reaches 7 gaps: no document
manages to omit all four top-level fields, `registrations`, `services`, and
`services[].version` simultaneously while still parsing as JSON with
*something* in it.

**Most common SHOULD gaps, ranked** (of 29,811 documents; a document can
and typically does contribute to several rows):

| Gap | Documents | % of 29,811 |
|---|---:|---:|
| `registrations` (absent or empty) | 24,697 | 82.8% |
| `type` | 15,808 | 53.0% |
| `services` (absent) | 13,120 | 44.0% |
| `image` | 12,458 | 41.8% |
| `services[].version` (≥1 entry) | 6,674 | 22.4% |
| `services_empty` (present, zero entries) | 5,200 | 17.4% |
| `description` | 72 | 0.2% |
| `name` | 20 | 0.1% |

Two things worth stating plainly rather than leaving implicit: first,
`description` and `name` are supplied almost universally (99.8% and 99.9%
of documents carry them) while `type` and `image` are each missing on
roughly half the population — the four fields Ruling 1 once treated as one
unit behave nothing alike in practice. Second, `services_status` splits the
29,811 documents as `absent`: 13,120 (44.0%), `empty`: 5,200 (17.4%),
`present` (non-empty): 11,491 (38.5%) — meaning **61.5% of documents that
otherwise parse as a valid agent registration file declare no way to reach
the agent at all**, whether by omitting the field or supplying it empty.
That number was invisible under the old fields_missing/fields_found shape,
which conflated "absent" and "empty" into the same `services` presence
check; FIX 3's `services_status` field is what makes it visible.

**Query methodology.** A standalone tool (not part of the committed
workspace — `crates/checks` stays free of any DB/network dependency, see
its purity discipline) linked the real `checks` crate as a path dependency
and re-judged each archived body through the actual `conformant()`
function, rather than re-deriving the MUST/SHOULD/MAY logic in SQL. This
was chosen over the SQL approach used in the FIX 1/2 entries above because
FIX 3's logic (nested per-entry aggregation, the services empty-vs-absent
split, the alias resolution) is materially more complex than a flat
field-presence list, and re-implementing it in SQL would have created a
second copy of the rule that could silently drift from the shipped code.
Internal consistency checks: `sum(should_gaps.len() over all docs)` equals
`sum(label frequency)` exactly (78,049 both ways); `services_status`
buckets sum to 29,811 exactly; the SHOULD-completeness histogram sums to
29,811 exactly; new-pass (29,552) equals `registrations`-absent-or-empty
documents (24,697, automatically zero MUST checks) plus
`registrations`-present documents with no missing sub-field
(5,114 − 259 = 4,855) — 24,697 + 4,855 = 29,552, confirming the two
independent tallies agree.

---

## 2026-07-29 — FIX 4: ungate rung 7 (formerly `independent`, now `attested`)

**Ships in the same run as FIX 5 and the rung-5 `unclaimed` status below —
not independently.** Ungating rung 7 without renaming it away from its old
self-review framing would have published the tautological finding FIX 5
describes at ~60,000-agent scale instead of the ~1,437 it was previously
computed over, which is strictly worse, not better. All three landed
together; this entry covers only the gating change.

**What changed.** Rung 7 used to be constructed only for agents that had
already passed rungs 1 through 5 (`reaches_rung7` in `crates/sweeper`
checked all five). It is now constructed for every agent that passes **rung
1 alone**. The ladder itself (`checks::run_ladder`) is restructured to match:
skip-propagation used to be one linear chain across all seven rung numbers,
so any failure below rung 7 — even one it has nothing to do with — silently
demoted it to `skipped`. That was true only by accident (the old gating
meant a rung-7 row was never actually present alongside a failing rung 2-5
in practice, so the bug never surfaced), and it stopped being safe to leave
in place the moment rung 7 started running unconditionally. `run_ladder` now
encodes three independent dependency tracks — **Document**
(1→2→3→4→5), **Service** (6, not yet implemented, depends on rung 4), and
**Reputation** (7, depends on rung 1 alone) — and propagates skip status only
within a track, never across one. See `crates/checks/src/ladder.rs`'s module
doc and `METHODOLOGY.md` §3 for the full graph.

**Why.** Reputation feedback lives in the Reputation Registry, keyed by
agent id, and `getClients`/`getSummary` are readable for any agent id that
exists on chain — regardless of whether that agent's *document* ever
resolved, parsed, conformed, or bound to it. There is no dependency between
the document track and the reputation track; the old gating measured "of the
~2% of agents whose document also happens to be perfect, how many have
outside feedback" — a hybrid question nobody asked and a number that
implicitly filtered out 97.6% of the population before the interesting
measurement even began.

**Cost, confirmed before shipping.** Rung 7 now reads feedback for
essentially the whole population (~60,000 agents) instead of ~1,437 — two
extra RPC calls per agent (`getClients`, then `getSummary` only if
`getClients` returned a non-empty list; `crates/chain/src/reputation.rs`
already guards this — confirmed unchanged, `getSummary` is never called with
an empty client array, which reverts on the deployed contract, see that
module's doc and its `#[ignore]`d live test
`empty_client_list_is_rejected_by_get_summary_not_treated_as_everyone`).

**Fixture, per the deliverable named explicitly by the work order:**
`crates/checks/src/ladder.rs::tests::rung_7_keeps_its_own_verdict_even_when_rung_2_fails`
constructs a ladder where rung 2 fails and rung 7 passes, and asserts rung 7
keeps its own `pass` verdict rather than being demoted to `skipped`. A
companion test, `rung_7_is_skipped_when_rung_1_fails`, confirms the one case
where it *is* skipped — rung 1 itself failing, its sole real dependency.

**Measured effect — the "not checked" collapse.** Cannot be measured against
the archived run: rung 7's chain read is live-only (`getClients`/
`getSummary`), and the archive holds only HTTP response bodies, not
Reputation Registry state. Reported as expected, not measured, consistent
with the work order's own instruction not to re-sweep for this fix:

> Rung 1 passes for **60,049 / 60,049 (100%)** agents in the reference run
> (`1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a` — see the FIX-5 entry below for how
> this run was confirmed as the current reference). Rung 7's old population
> was exactly the old rung-5 pass count, **1,437**. Under this fix, the
> expected new denominator is **~60,049**, a ~42x increase. The work order's
> own estimate (~60,037, from an earlier, non-reference run) and the
> "not-checked count falls to zero except ~12 hard chain-read errors" figure
> are both stated by the work order as reference numbers from a run this
> fix does not re-run; they are not re-derived here.

---

## 2026-07-29 — FIX 5: rename rung 7 to `attested`; drop the self-review framing

**Ships in the same run as FIX 4 above — see that entry's opening note.**

**What changed.**
1. Rung id `independent` → `attested`, everywhere: `crates/checks` (module
   file renamed `rung7_independent.rs` → `rung7_attested.rs`,
   `IndependentInput` → `AttestedInput`, `independent()` → `attested()`),
   `METHODOLOGY.md` §2, `CHANGELOG-METHODOLOGY.md` (this file), the sweeper's
   log/doc comments, and the frontend's `/methodology` page and status
   styling (`agentcount-web`, then named `ledgerscope-web`).
2. The rung no longer compares feedback authors against the agent's owner at
   all. `AttestedInput` no longer carries an `owner` field. Verdict logic
   simplifies to: `Pass` if `getClients` returns ≥1 distinct address, `Fail`
   (`no_feedback`) if it returns zero, `Error` (`no_reputation_registry`) if
   the chain has no Reputation Registry — unchanged from before.
3. Evidence drops `authors_equal_to_owner` and the derived
   `self_feedback_ratio` entirely — not renamed, removed. `feedback_count`
   and `distinct_authors` are unchanged.
4. Sybil/coordination analysis (funding-linked, deployer-linked,
   operator-adjacent addresses) is confirmed as **inference that belongs in
   a separately-labelled `signals` block, never a rung result** — and is
   **not built as part of this fix**, per the work order's explicit
   instruction.

**Why — verified against the pinned spec.** `spec/ERC8004SPEC.md` line 217:
*"The feedback submitter MUST NOT be the agent owner or an approved operator
for `agentId`."* This is a contract-level invariant: feedback from the
owner's own address cannot be successfully submitted. The old `independent`
rung computed `authors_equal_to_owner` regardless and reported "zero agents
caught writing their own reviews" as a floor-check finding. It is not a
finding — its only possible outcome (short of a bug) is restating a rule
nobody can break — and publishing it at the ~60,000-agent scale FIX 4
ungates rung 7 to would have been publicly corrected within a day. The spec
also names the underlying open problem it does not solve (line 324):
`getSummary` requires a non-empty `clientAddresses` filter precisely because
unfiltered results are subject to Sybil/spam attacks, and expects
reviewer-reputation to emerge off-chain — a different, harder problem than
this rung, or this project's ladder, is built to answer.

**Verification: no output anywhere claims agents were checked for
self-review.** Grepped the full committed tree, case-insensitive, for
`self.review`, `self_feedback`, and every remaining use of `independent` as
applied to rung 7:
- `self_feedback_ratio` / `authors_equal_to_owner`: zero remaining
  occurrences in `crates/checks`, `crates/sweeper`, `crates/chain`,
  `crates/api`, `METHODOLOGY.md`, or the frontend — removed, not renamed.
- `independent` as rung 7's identifier: zero remaining occurrences in code
  (`rung: 7, name: "attested"` everywhere the rung is constructed) or in
  `/methodology`'s prose; the word survives only in unrelated senses (this
  project's own tagline "independent conformance and census layer",
  `crates/chain`'s unrelated "independently readable" comments, an unrelated
  "independent of overall sweep concurrency" note, and — deliberately kept,
  as history — the retrospective "renamed from `independent`" sentences in
  `METHODOLOGY.md` and this changelog explaining what changed and why).
- `rung7_attested`'s module doc states plainly, as prose a reader does not
  have to infer from field absence: *"This rung does not, and cannot, detect
  self-review — no evidence field, log line, or report copy produced here
  claims otherwise."*

**A finding that reinforces the decision, found while writing this entry.**
Querying the archived run's stored (pre-fix) rung-7 evidence directly
(`evidence->>'authors_equal_to_owner'`, run `1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a`,
1,437 rows): **1,436 of 1,437 read `0`; exactly one — agent 51120 — reads
`1`** (`{"feedback_count": 14, "distinct_authors": 14,
"self_feedback_ratio": 0.0714..., "authors_equal_to_owner": 1}`). This is
not a spec violation caught in the wild: it almost certainly reflects
ownership *changing* after the feedback was left (an ERC-721 transfer, e.g.
a marketplace sale, moves `ownerOf` to a new address without touching
already-recorded feedback) — the spec's MUST is enforced against the owner
*at submission time*, not the owner we happen to read at the pinned block of
a later census. "Current owner equals a past feedback author" is therefore
not evidence of self-review either, which is a second, independent reason
`authors_equal_to_owner` was never a metric worth keeping: even its rare
non-zero values are not interpretable as the self-review signal its name
implied.

**`self_feedback_ratio`: dropped, not renamed — decision and justification.**
Considered renaming it (e.g. to something like `owner_author_fraction`) to
preserve it as "evidence about author composition" rather than a verdict,
per the precedent this project sets elsewhere (evidence fields are kept even
when they don't gate pass/fail). Decided against, for two independent
reasons: (1) since owner self-feedback is contract-level impossible, the
ratio is, empirically, a near-constant `0.0` (1,436 of 1,437 in the archived
run) — a measurement that can only ever have one value is not evidence, it
is decoration, and keeping it under any name would just move the tautology
FIX 5 removes from the verdict into the evidence block instead of actually
removing it; (2) the one non-constant value found (agent 51120, above) is
not interpretable as self-review either, because ownership can change after
feedback is recorded — so even a genuinely non-zero ratio would not mean
what its name claims. Both `authors_equal_to_owner` and
`self_feedback_ratio` are removed from rung 7's evidence entirely.
`feedback_count` and `distinct_authors` are kept unchanged: both remain
genuine, recomputable, per-agent measurements that say nothing about author
identity relative to the owner.

**Measured effect on verdicts.** A rename plus an evidence-field removal
does not, by itself, guarantee no agent's verdict moves: the new `attested()`
pass condition (`distinct_authors ≥ 1`) is a strict *widening* of the old
`independent()` pass condition (`distinct_authors ≥ 1` AND at least one
client ≠ owner) — an agent whose old reason was `only_self_feedback`
(feedback existed, but every author happened to equal the owner) would flip
from `Fail` to `Pass` under the new logic. Checked directly against the
archived run's stored rung-7 evidence rather than assumed: **zero of the
1,437 agents carry `reason: "only_self_feedback"`** (620 carry
`no_feedback`; the remaining 817 already carry no `reason` at all, i.e.
already `Pass`). So for this specific archived run, the verdict split is
unchanged — 817 pass, 620 fail, both under either rung's logic — and only
the evidence shape (dropped fields, renamed id) changed. This is reported as
an empirical fact about this run, not a guarantee: a future run could
contain an `only_self_feedback` agent (however unlikely per the
contract-invariant discussion above) and would see it flip. The real
population effect at full scale is FIX 4's, not this one's — see that entry.

---

## 2026-07-29 — NEW: rung 5 (`bound`) gains a fifth status, `unclaimed`

**What changed.** `CheckStatus` gains a fifth variant, `Unclaimed`
(`crates/checks/src/model.rs`), produced only by rung 5. `bound()`
(`crates/checks/src/rung5_bound.rs`) now returns `Unclaimed` — evidence
`reason: "unclaimed"`, `match: null`, `registrations_seen: 0` — for a
document with no `registrations` array, a `null` one, a non-array value, or
a present-but-empty array. It previously returned `Fail` with
`reason: "no_registrations"` for all four of those shapes; `match` was
`false`. Matching entries still `Pass`; a present, non-empty array with no
matching entry still `Fail` — both unchanged in every particular, including
evidence shape.

This is a schema change: a new `status` value plus new evidence semantics
for one existing field (`match` can now be `null`, not only `true`/`false`).
`schema_version` bumps 2 → 3 and `checker_version` bumps 0.2.0 → 0.3.0
(`crates/checks/Cargo.toml`) — the same version bump covers this change and
FIX 4/5 above, since the work order requires all three to ship in one run.
Migration `0011_rung5_unclaimed_status.sql` drops and re-adds
`check_results_status_check` to accept `'unclaimed'` alongside the original
four values (`ALTER CONSTRAINT` does not exist in Postgres). `crates/api`'s
`VALID_STATUSES` (the `status=` query-parameter allowlist on
`GET /api/agents`) grows from four entries to five, so a client sending
`status=unclaimed` gets real filtering rather than a 400. The frontend
(`agentcount-web`, then named `ledgerscope-web`) status-to-colour mapping (`lib/status.ts`) and the
`/methodology` page are updated so an unrecognised status still renders
(neutral styling, verbatim text) rather than being guessed at as `pass` or
`fail`.

**Why.** P0 FIX 3 (above, 2026-07-29) reclassified `registrations` from an
unconditional requirement to SHOULD, which means a document can pass rung 4
while declaring zero registrations — and rung 5's entire question ("does the
document's own registration entry match the on-chain record we fetched it
from") has nothing to check in that case. None of the four original statuses
was honest for it: `pass` would claim a verification that never happened;
`fail` would punish a merely-recommended field exactly as hard as a genuine
on-chain mismatch, collapsing two different failure modes ("said nothing"
vs. "said the wrong thing") into one word; `skipped` would falsely imply an
earlier rung failed (rung 4 passed); `error` would falsely imply this
checker malfunctioned (it did not — it correctly found nothing to verify).

**Measured effect — re-judged against the archived run, not re-swept.**
Verified the current reference run first, per the work order's instruction
not to trust the run id it named: queried `runs` for the most recent
completed run and got `1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a` (finished
2026-07-28 19:13:29, 60,049 agents) — **not** `c817ab28-8157-4925-93d6-2a6e0610020d`
(an earlier, superseded run, finished 2026-07-28 14:19:15, 60,037 agents)
that the work order's own text named. `1c87c4f4` is also the run every prior
2026-07-28 entry in this file already measures against, so this entry stays
consistent with them. A standalone tool (not part of the committed
workspace, same discipline as the FIX-3 entry above — `crates/checks` stays
free of any DB/network dependency) linked the real `checks` crate as a path
dependency, re-judged every archived body against the CURRENT
`conformant()` and `bound()` functions directly (not SQL logic re-derived
from them), and guarded body decoding in two stages — `String::from_utf8`,
then `serde_json::from_str` — counting failures of either stage separately
rather than aborting (the SQL-equivalent of `convert_from(body,'UTF8')::jsonb`
guarded per row). Population: the 29,811 documents where the STORED rung 3
(`parseable`) verdict already passed (joined from `check_results`, not
re-derived from "does the body merely parse" — those two sets differ by
exactly 253, the same 253 documents recorded as rung-4 `skipped` in the
stored run, where a body parses as JSON but rung 3 still correctly recorded
`error` because the archived body was truncated; matching FIX 3's own
population scoping avoids silently re-including those 253). Zero of those
29,811 bodies were non-UTF8 or non-JSON, consistent with rung 3 having
already validated them.

> **Re-judged rung 4: 29,552 pass, 259 fail** — exactly matching the FIX-3
> entry's own re-judged figures against the same archived run, confirming
> internal consistency across the two independent tools.
>
> **New rung-5 split, population = 29,552 (the re-judged rung-4-pass set):**
>
> | Status | Count | % of 29,552 |
> |---|---:|---:|
> | `pass` | 4,055 | 13.7% |
> | `fail` | 800 | 2.7% |
> | `unclaimed` | 24,697 | 83.6% |
>
> Against the **old** figures (population 4,175, the old rung-4-pass count
> under the pre-FIX-3 required-field list): **1,437 pass (34.4%), 2,738 fail
> (65.6%), 0 unclaimed** (the status did not exist yet; every absent-claim
> document was counted as `fail`). The denominator grew **~7.1x** (4,175 →
> 29,552), exactly the "~7x larger" the work order anticipated. The 556
> agents that flip in the FIX-3 measurement of "`registrations`-present with
> no missing sub-field" minus rung-5-fail context are superseded here by a
> direct re-judgement: the 24,697-document `unclaimed` figure is the same
> population size as the FIX-3 entry's independently-measured
> `registrations` (absent or empty) SHOULD-gap count of 24,697 — the two
> tools agree exactly, which is the strongest available check that this
> entry's population scoping and FIX 3's are the same set.

The absolute pass count rose (1,437 → 4,055) even though the pass *rate*
fell (34.4% → 13.7%): the new population is dominated by documents that
declare no registration at all (`unclaimed`, 83.6%) — the majority of the
25,377 agents FIX 3 newly admits to rung 4's pass set are exactly the
agents that omit `registrations`, which is the SHOULD FIX 3 stopped
penalizing. Read together with FIX 3's SHOULD-completeness table: **most of
the population that now "passes conformance" is passing in part *because*
it declines to make a binding claim rung 5 could check** — `unclaimed` is
what makes that pattern visible instead of silently absorbed into `fail`.

**Fixtures**, one per the four behaviours the work order named explicitly
(`crates/checks/src/rung5_bound.rs::tests`):
`absent_registrations_is_unclaimed_not_a_fail`,
`empty_registrations_array_is_unclaimed_not_a_fail`,
`exact_match_passes` (already existed, unchanged), `wrong_agent_id_fails`
(already existed, unchanged) — plus
`a_registrations_value_that_is_not_an_array_is_unclaimed_not_panics` for the
non-array-value edge case (a string, a number, a bare object, a boolean).

---

## 2026-07-29 — FIX 6: `absent` vs `skipped` for rungs 4 and 5

**What changed.** `crates/sweeper/src/main.rs` used to construct rung 4
(`conformant`) and rung 5 (`bound`) only when rung 3 (`parseable`) had
actually handed back a parsed document (`document.as_ref().map(...)`, one
`Option` gate per rung). When there was nothing to parse — the agent's
`tokenURI()` never resolved, the fetch errored, or the body fetched fine but
was not valid JSON — rungs 4 and 5 were simply left out of the vector handed
to `checks::run_ladder`. `run_ladder` never saw a row for them, so it had
nothing to mark `skipped`; the agent ended up with **no rung-4/5 row at
all**, identical in shape to rung 6, which genuinely is not implemented.
`METHODOLOGY.md` §4 defines `absent` as "not yet checked" and `skipped` as
"a lower rung did not pass, so this question could not be meaningfully
asked" — the two were being conflated for every agent this happened to.

The fix moves the decision entirely into a new pure function,
`assemble_ladder` (`crates/sweeper/src/main.rs`): rungs 4 and 5 are now
**always constructed** — passed a placeholder (`serde_json::Value::Null`)
document when none exists — and **always** included in the vector handed to
`checks::run_ladder`. No `Skipped` status is ever computed in the sweeper;
`run_ladder` (`crates/checks/src/ladder.rs`, untouched by this fix) is the
only place that decides it, exactly as before. This is safe because
`document` is `None` if and only if rung 3's own status is non-`Pass` (see
`rung3_parseable`'s return type — it only ever hands back `Some(document)`
on its `Pass` path), which means rung 3 is *always* already acting as a
stopper in `run_ladder`'s dependency graph whenever the placeholder would
otherwise matter; whatever `conformant`/`bound` compute from
`Value::Null` is unconditionally overwritten with `Skipped` before
`assemble_ladder` returns. Rung 6 is unaffected — it is still never
constructed, so "no row" remains the correct and only signal for it.

No evidence shape changed. `skipped` rows for rungs 4 and 5 carry the same
`skipped_because_rung`/`skipped_because_status` fields `run_ladder` has
produced since P0 FIX 4/5 (crediting the *original* stopper in a track, not
just the immediate parent) — this fix only makes those rows exist for the
agents that were missing them. No `SCHEMA_VERSION` or `CHECKER_VERSION`
bump: the `check_results` table's shape, the set of valid `status` values,
and the evidence contract are all unchanged; only *coverage* of the
existing `skipped` status changes.

**Why.** Identified internally (not from the external review) as part of
auditing the reference run's row counts before publication: rung 3 shows
`skipped` for every one of its dependency's non-`pass` agents, but rungs 4
and 5 show `skipped` for only a few hundred of the same population — a
mismatch that is only explainable by rungs 4/5 not reaching `run_ladder` at
all for most of them. Distinguishing "we did not ask" from "we could not
ask" is close to the whole premise of this project (see `METHODOLOGY.md`
§4); silently blurring an implemented-but-blocked rung into the same shape
as an unimplemented one is exactly the kind of compression this project
exists to refuse, and it is the first thing an external reviewer would
flag next.

**Verified against `checks::ladder`'s existing tracks before changing
anything** (ground rule: "read the current `run_ladder` ... before changing
anything, make sure your change preserves that tracks are independent"):
rungs 1→2→3→4→5 are the Document track; rung 7 (`attested`) depends on rung
1 alone, on its own Reputation track. `assemble_ladder` passes `rung7`
through unchanged and unconditionally alongside rungs 1–5; skip-propagation
inside `run_ladder` only ever follows `depends_on`, which has no edge from
the Document track into the Reputation track. Confirmed by a new fixture
below.

**Fixtures** (`crates/sweeper/src/main.rs::tests`, exercising the new
`assemble_ladder` function directly — pure once its inputs are in hand, no
database or RPC needed):
- `a_rung_2_failure_skips_rungs_3_4_and_5_and_never_touches_attested` — the
  deliverable fixture verbatim: rung 2 fails → rungs 3, 4, 5 all present and
  `skipped`, each carrying `skipped_because_rung: 2`; rung 7 (`attested`)
  stays `pass` and carries no skip evidence at all; rung 6 stays absent.
- `a_rung_4_failure_skips_rung_5_naming_rung_4_as_the_blocker` — a document
  that parses and clears rung 4's SHOULD checks but violates its one MUST
  (a `registrations` entry missing `agentRegistry`) fails rung 4 for real
  (not overwritten — rung 3 passed, so rung 4's own dependency is satisfied)
  and skips rung 5 with `skipped_because_rung: 4`; rung 6 stays absent.
- `a_fully_passing_document_track_is_not_skipped_anywhere` — a fully-passing
  chain leaves rungs 4 and 5 as real `pass` verdicts, confirming the
  unconditional construction does not turn a healthy agent into one that
  looks blocked.

`crates/checks` was re-read, not modified: `run_ladder` and its own test
suite (`crates/checks/src/ladder.rs`) already correctly implement
track-scoped skip-propagation (added by P0 FIX 4/5) and are unchanged by
this fix, per the work order's instruction that `run_ladder` alone owns
this logic. Purity check unaffected: `grep -RniE
'reqwest|sqlx|alloy|tokio|Utc::now' crates/checks/` is still empty.

**Measured effect — swept the stored reference run
(`1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a`, 60,049 agents) to confirm the
*shape* the fixed sweeper would produce, not re-judged and not re-swept**
(re-judging rung 4/5 verdicts themselves is unaffected by this fix — it
only changes whether an already-computed verdict, or a skip, gets a row at
all):

> **29,985 agents currently have NO rung-4 row and NO rung-5 row at all** —
> the full defect population, computed directly (`NOT EXISTS` against
> `check_results` per agent), not estimated. Breaking this down by *what
> actually blocked them* (the `blocked_by`/`skipped_because_rung` value each
> would carry once fixed):
>
> | Original stopper | Rung-2 status | Agents |
> |---|---|---:|
> | Rung 2 | `fail` | 25,242 |
> | Rung 2 | `error` (our fault, not the agent's) | 2,686 |
> | Rung 3 (rung 2 itself passed) | `fail`/`error` at rung 3 | 2,057 |
> | **Total** | | **29,985** |
>
> The work order's own headline figure, **25,242 agents**, is exactly the
> first row above — verified, not assumed: of the 25,495 agents whose rung 2
> is `fail`, 253 already have a (correctly `skipped`) rung-4 row today, by
> an accident of the old gating (their rung 3 body happened to still parse
> as JSON even though rung 2 itself failed, so `document.as_ref().map(...)`
> still constructed rung 4/5 for them, which `run_ladder` then correctly
> skipped from rung 2's own failure); 25,495 − 253 = 25,242 do not.
>
> **This undercounts the true defect population by 4,743 agents** (2,686 +
> 2,057) — reported here because ground rule 1 requires disagreements
> surfaced, not silently resolved. The work order's framing ("for agents
> failing rung 2") describes only the largest of three causes; the actual
> code defect is "any agent whose rung 3 does not hand back a document,"
> which also includes 2,686 agents where rung 2 itself **errored** (our
> fault, not a claim about the agent — these were already being under-
> counted as an omission, not a `fail`, so no direction of the census's
> bias flips, but the absent-row bug affected them exactly as much as the
> `fail` cases) and 2,057 agents where rung 2 **passed** but rung 3 itself
> failed or errored (a body that fetched but was not valid JSON, or was
> truncated) — a case the work order's "failing rung 2" framing does not
> mention at all, since rung 2 is not what blocked these.
>
> **Shape after the fix** (same archived facts, re-classified — not a
> rerun): rung 4 goes from 4,175 `pass` / 25,636 `fail` / 253 `skipped` /
> 29,985 absent to 4,175 `pass` / 25,636 `fail` / **30,238 `skipped`** / 0
> absent (253 + 29,985). Rung 5 goes from 1,437 `pass` / 2,738 `fail` /
> 25,889 `skipped` / 29,985 absent to 1,437 `pass` / 2,738 `fail` /
> **55,874 `skipped`** / 0 absent (25,889 + 29,985). Both totals reconcile
> to 60,049, the full swept population, for the first time — previously
> only rung 1 (60,049), rung 2 (60,049), and rung 3 (60,049, all present
> whether `pass`, `fail`, `error`, or `skipped`) had every agent accounted
> for at every rung.
>
> **The archived run is not rewritten.** Runs are immutable
> (`METHODOLOGY.md` §5); `1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a` keeps its
> old shape — 29,985 absent rung-4/5 rows and all — as the honest record of
> what the pre-FIX-6 checker actually produced, including the part where it
> was wrong. The corrected shape above takes effect starting with the next
> sweep run only.

**Fixtures verify no cross-track leakage.** The fixture
`a_rung_2_failure_skips_rungs_3_4_and_5_and_never_touches_attested` asserts
`attested` stays `pass` with no skip evidence when rung 2 fails — confirming
the FIX 4/5 track boundary (Document vs. Reputation) survives this change,
per the work order's explicit warning that a rung-2 failure must never mark
`attested` skipped.

---

## 2026-07-29 — FIX 7: `data:` URI coverage (five decode fallback paths)

**What changed.** `crates/probe`'s `data:` URI decoder (`resolve.rs`) tries
five paths, in order, recording which one succeeded:

1. `enc=<algorithm>[;level=<n>]` present — decompress with the named
   algorithm, then use the result. **Only `gzip` is implemented.**
2. Any `;base64,` meta at all, regardless of declared MIME type or charset
   (`data:text/plain;base64,`, `data:;base64,`,
   `data:application/json;charset=utf-8;base64,` all decode identically) —
   plain base64 decode. This path already existed before this fix; it is
   unchanged, just now explicitly named and fixtured.
3. No `;base64,` token at all — literal/percent-encoded text. Also
   pre-existing and unchanged; explicitly named and fixtured.
4. A payload that *claims* base64 but plainly starts with `{` or `[` — the
   decode is skipped and the payload used as-is.
5. No `data:` scheme at all — the on-chain URI string itself is raw JSON
   (`{...}`/`[...]`), treated as an already-in-hand payload the same as any
   `data:` URI's decoded bytes.

Every fallback path is recorded in rung 2's evidence
(`data_uri_variant`, one of `"compressed"`, `"base64"`, `"plain"`,
`"base64_claimed_but_raw_json"`, `"plain_json_without_uri_scheme"`; plus
`data_uri_algorithm` when compression was involved) — which path succeeded
is itself data worth publishing, per the work order.

**A `data:` URI declaring a compression algorithm this parser does not
implement is `checks::CheckStatus::Error`, never `Fail`.** We understood
exactly what was declared (the `enc=` value is recorded verbatim in the
`error` reason, `"unsupported_compression: <algorithm>"`), we simply cannot
decode it — that is our limitation, not a defect in the agent's document.
This is a new `Target::UnsupportedCompression` classification in
`crates/probe`, distinct from the pre-existing `Target::Unsupported` a
genuinely malformed `data:` URI (no comma separator, base64 that fails to
decode and isn't the raw-JSON case above) still produces — that case is
unchanged and still `Fail`.

`ipfs://` classification moved out of `resolve()` entirely, into a new
`ipfs_cid_and_path()` — a mechanical consequence of FIX 8 below (picking a
gateway now takes live network attempts, which `resolve()` deliberately
never makes), not a FIX 7 change in its own right.

**Why.** The ecosystem's documented convention is
`data:application/json;enc=<algorithm>[;level=<n>];base64,<payload>` with
zstd, gzip, brotli, or lz4 compression, plus several non-standard variants
observed in production. The first census's decoder only handled plain
base64 and literal payloads; an `enc=`-compressed payload base64-decoded
into opaque binary that then failed rung 3's JSON parse — reporting our own
missing decompression support as the agent's malformed document.

**Measure first, then implement (per the work order's explicit
instruction).** Queried `agent_snapshots.agent_uri` directly for the
reference run (`1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a`, 60,049 agents; no
re-sweep) for every variant named in the work order, before writing any
decompression code:

| Variant | Population count |
|---|---:|
| `enc=` parameter present | 399 (all declare `gzip` — zero `zstd`, `brotli`, or `lz4`) |
| `data:text/plain;base64,` | 0 |
| `data:;base64,` (no MIME) | 0 |
| `data:application/json;charset=utf-8;base64,` | 0 |
| Plain (non-base64) `data:` URI | 50 |
| Base64 meta whose payload is actually raw JSON | 0 |
| Raw JSON, no `data:` scheme at all | 127 |

**zstd/brotli/lz4: implemented as classification only, no dependency
added.** zstd is the ecosystem's *recommended* algorithm, yet it has zero
occurrences in 60,049 agents, as do brotli and lz4. Adding `zstd`'s (or
brotli's, or lz4's) decoding crate to decode zero real documents was
deliberately not done, per the work order's explicit instruction — an
`enc=` value of any of these three (or any other, unrecognised string) is
classified `UnsupportedCompression` (evidence-only, no decode attempted) and
reported here rather than silently added. **Only `gzip` — the one algorithm
that actually appears — got a real dependency (`flate2`).** The three
already-working paths (plain base64 regardless of declared MIME/charset,
literal/percent-encoded payloads, and the base64-claims-raw-JSON fallback)
were cheap to implement/verify and are fixtured even where the reference
population shows zero occurrences (the base64-claims-raw-JSON case), per
the work order's "implement it anyway, note it's untested against real
data" instruction.

**Measured effect — re-judged against the archived response bodies for the
reference run (60,049 agents; no re-sweep).** Rung 3 (`parseable`) currently
fails 2,046 documents; broken down by `http_archive.scheme` and, within
`scheme = 'data'`, by the exact URI pattern:

| Population | Count | Disposition under this fix |
|---|---:|---|
| `enc=gzip` (base64-decodes to valid gzip, decompresses to valid JSON — verified for all 399, not sampled) | 399 | **flips `fail` → `pass`** |
| `data:`-scheme fails NOT explained by `enc=` (base64-decodes cleanly to non-JSON bytes — literal `-n {...}` shell-quoting artifacts in the payload, an unrelated defect in whatever tool produced them) | 76 | unchanged — genuinely malformed, correctly stays `fail` |
| `https:`-scheme fails (malformed bodies from web fetches — outside this fix's scope entirely) | 1,566 | unchanged |
| `ipfs:`-scheme fails | 5 | unchanged |
| **Total rung-3 fails accounted for** | **2,046** | matches exactly |

> **399 of the 2,046 rung-3 failures (19.5%) are attributable to
> unsupported gzip compression and flip to `pass` under this fix.** The
> remaining 1,647 are genuine malformed-document failures (76 `data:`-scheme,
> 1,566 `https:`-scheme, 5 `ipfs:`-scheme), unaffected by this fix and
> correctly still `fail`.

**A second, larger population effect this fix produces, outside the
"rung-3 failures" framing the work order used:** the 127 raw-JSON-no-scheme
agents currently fail **rung 2** (`unsupported_scheme`), not rung 3, because
today's parser never recognizes a scheme-less string as a document at all.
Checked each of the 127 raw `agent_uri` strings directly (not sampled):

> **105 of 127 (82.7%) are themselves valid JSON** and flip from rung-2
> `fail` all the way to rung-2 `pass` **and** rung-3 `pass` under this fix.
> **22 of 127 (17.3%) are not valid JSON** (truncated strings, an
> unescaped/embedded control character, or a bare bracketed URL like
> `[https://gmgn.ai/]`) — these flip from rung-2 `fail` (`unsupported_scheme`)
> to rung-2 `pass` / rung-3 `fail` (a real `parse_error`, correctly
> attributed to the agent's document, not this project's parser
> limitation).

The 50 plain (non-base64) `data:` URIs already passed both rung 2 and rung
3 before this fix (verified against the archived run) — that fallback path
already existed; this fix adds no population change for it, only the
explicit `data_uri_variant: "plain"` label and a dedicated fixture.

**Schema.** `schema_version` bumps 3 → 4: rung 2's evidence gains
`data_uri_variant`/`data_uri_algorithm` for a `"data"`-scheme result.
`checker_version` bumps 0.3.0 → 0.4.0 (`crates/checks/Cargo.toml`);
`probe`'s own crate version bumps 0.2.0 → 0.3.0 (`crates/probe/Cargo.toml`)
for the `Target`/`FetchOutcome` public shape change this fix makes. No
migration is needed: no `status` value changed, only new `jsonb` evidence
keys. FIX 8 (next entry) bumps both again for its own evidence addition.

**Fixtures**, one per variant named in the work order plus the two `error`
cases (`crates/probe/src/resolve.rs::tests` unless noted):
`a_gzip_compressed_data_uri_decompresses_before_parsing`,
`an_unsupported_compression_algorithm_is_error_not_fail` (zstd/brotli/lz4/an
invented name, all four),
`any_base64_meta_variant_decodes_regardless_of_mime_or_charset` (all three
non-standard MIME/charset variants), `a_plain_non_base64_data_uri_is_url_decoded`,
`a_base64_meta_whose_payload_is_actually_raw_json_skips_the_decode`,
`raw_json_with_no_uri_scheme_is_inline_with_a_named_variant`. Plus, in
`crates/probe/src/fetch.rs::tests` (the public `Prober::fetch` entry
point): `an_unsupported_compression_algorithm_is_error_not_fail_and_touches_no_network`,
`a_gzip_compressed_data_uri_is_decoded_via_the_public_fetch_entry_point`.
Plus, in `crates/checks/src/rung2_resolvable.rs::tests` (the `Error`, not
`Fail`, verdict at the check level): `an_unsupported_data_uri_compression_algorithm_is_error_not_fail`,
`a_gzip_compressed_data_uri_passes_and_records_the_algorithm`.

**Purity check unaffected:** `grep -RniE
'reqwest|sqlx|alloy|tokio|Utc::now|probe' crates/checks/` (dependency
names, not doc-comment mentions) is still empty — the new evidence fields
are plain `String`/`Option<serde_json::Value>` inputs assembled by
`crates/sweeper`, exactly like every other field `ResolvableInput` already
carried.

---

## 2026-07-29 — FIX 8: IPFS gateway fallback chain

**Reverses a 2026-07-28 ruling, not a silent rewrite.** That ruling chose
one disclosed gateway (`ipfs.io`) specifically so a failure fetching through
it would be honestly attributable — the report said so explicitly ("no
second gateway was tried"). The owner has confirmed the reversal: a single
gateway's own outage was itself becoming indistinguishable from an agent's
content being genuinely unpinned, which defeated the original goal rather
than serving it.

**What changed.** `crates/probe::Prober` now holds a list of IPFS gateways
(`ipfs_gateways: Vec<String>`, changed from a single `String`) and tries
each in sequence — `https://ipfs.io/ipfs/`, `https://cloudflare-ipfs.com/ipfs/`,
`https://gateway.pinata.cloud/ipfs/`, in that order — until one answers HTTP
2xx or all three are exhausted. Each attempt goes through the exact same
guarded path as any other fetch (`fetch_http`: SSRF netguard, `robots.txt`,
redirects, the body cap) — nothing about politeness is relaxed or
special-cased for gateways. `crates/sweeper`'s `ipfs_gateway()` (singular)
becomes `ipfs_gateways()` (plural), overridable via `IPFS_GATEWAYS`
(comma-separated) instead of `IPFS_GATEWAY`.

`ipfs://` classification moved out of `crates/probe::resolve()` (which
stays synchronous and network-free by design) into a new, separate
`resolve::ipfs_cid_and_path()` plus a new `Prober::fetch_ipfs_chain` that
drives the actual multi-gateway attempt sequence — picking which gateway
serves an agent, and trying more than one, is inherently a live-network
decision that a pure classifier cannot make.

**Evidence records the whole chain, not just the winner.** `FetchOutcome`
gains `gateway_attempts: Vec<GatewayAttempt>` (`gateway`, `http_status`,
`error` per attempt), which rung 2 surfaces verbatim as
`evidence.gateway_attempts`; `via_gateway` still records which one (if any)
won. A reader can now see "gateway 1 404'd, gateway 2 timed out, gateway 3
answered 200" in full, not just the winning URL.

**All three gateways failing is `checks::CheckStatus::Error`, never
`Fail`.** This is a deliberate divergence from how a plain `https://`/`http://`
fetch is judged (where a definite non-2xx status, e.g. a real 404 from the
origin, is `fail` — the origin answered and said no): for `ipfs://`, we
cannot tell an unpinned CID (the agent's own document is genuinely gone)
from a gap in what our three chosen gateways happen to have cached (our
limitation) — claiming otherwise would be a claim this project cannot
support. Mechanically, this falls out of the existing `error`-vs-`fail`
branch in `checks::resolvable` for free: `fetch_ipfs_chain` sets
`FetchOutcome.error = Some("ipfs_all_gateways_failed")` whenever no gateway
answers 2xx (regardless of what individual, non-2xx statuses each one
returned), and that string carries no `ssrf_blocked:` prefix, so rung 2's
already-existing rule ("an `error` without that prefix is OUR limitation")
applies unchanged — no new branch was added to `crates/checks` to make this
true.

**Per-host concurrency cap still applies, per gateway host.** `guarded_send`'s
semaphore is keyed by `url.host_str()`, unchanged by this fix; three
gateways are three different hostnames, so each gets its own independent
cap automatically — nothing bypasses or shares the budget across them. No
new test was written FOR this specific guarantee beyond code inspection: the
existing `no_more_than_two_requests_are_ever_concurrently_in_flight_against_one_host`
test already exercises the exact mechanism (`host_semaphore`) this fix
reuses unchanged, just called three times (once per gateway host) instead
of once.

**Why.** Single gateway makes a gateway outage indistinguishable from an
agent's failure — exactly the ambiguity the 2026-07-28 ruling's own
one-gateway choice was meant to make *honestly attributable*, not resolve.
A three-gateway fallback chain narrows that ambiguity for any agent whose
content is pinned to *at least one* of the three, while the all-three-fail
case still honestly reports `error` rather than pretending to have
resolved the remaining ambiguity.

**Population this affects.** Queried `agent_snapshots.agent_uri` for the
reference run (`1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a`, 60,049 agents; no
re-sweep):

> **3,588 agents (5.98%) declare an `ipfs://` `tokenURI()`** (the work
> order's own figure, 3,587, is one agent off this direct count — reported
> as measured, not adjusted to match). Of those, under the *pre-fix*
> single-gateway checker: **2,946 pass** rung 2 already (the single
> `ipfs.io` gateway answered 2xx), **518 fail** (`http_status`, a non-2xx
> from `ipfs.io`), and **124 error** (`timeout`, `ipfs.io` itself did not
> answer in time). The 518 + 124 = **642 agents are the population this fix
> can move** — each is a candidate whose content might be pinned on
> Cloudflare's or Pinata's gateway even though `ipfs.io` didn't serve it.

**Not re-swept, and correctly so.** The archive holds only the bodies and
statuses `ipfs.io` actually returned for this run; it has no record of what
`cloudflare-ipfs.com` or `gateway.pinata.cloud` would have answered for the
same 642 agents at the same historical moment; a live fetch today would
also no longer reflect the block this run pinned to. Which of the 642 flip
to `pass`, and how many of the 2,946 already-passing agents turn out to have
been reachable via a *different* gateway than `ipfs.io` all along (visible
only once `gateway_attempts` starts being recorded), is reported at the
next full census rerun, not estimated here.

**Schema.** `schema_version` bumps 4 → 5 (on top of FIX 7's 3 → 4, above):
rung 2's evidence gains `gateway_attempts` for an `"ipfs"`-scheme result.
`checker_version` bumps 0.4.0 → 0.5.0 (`crates/checks/Cargo.toml`); `probe`'s
own crate version bumps 0.3.0 → 0.4.0 (`crates/probe/Cargo.toml`) for the
`Prober::new` signature change (a single gateway string to a list) and the
new `FetchOutcome.gateway_attempts`/`GatewayAttempt` public API. No `status`
value changed; no migration needed — only new `jsonb` evidence keys.

**Fixtures** (`crates/probe/src/fetch.rs::tests`):
`an_ipfs_fetch_falls_back_to_the_second_gateway_when_the_first_fails` (first
gateway 404s, second answers 200 — the deliverable fixture verbatim),
`ipfs_all_gateways_failing_is_error_never_fail` (all three 404, asserts
`error: "ipfs_all_gateways_failed"`, never a bare status, with all three
attempts recorded), `the_first_gateway_succeeding_never_calls_the_others`
(the second and third gateways carry no mock at all; the assertion is on
`gateway_attempts.len() == 1`, proving neither was ever reached). Plus, at
the check level (`crates/checks/src/rung2_resolvable.rs::tests`):
`ipfs_pass_records_the_whole_gateway_chain_not_just_the_winner`,
`all_three_ipfs_gateways_failing_is_error_not_fail_with_the_full_chain_recorded`.

**`METHODOLOGY.md` updated, not silently rewritten** — Section 2 (Rung 2)
now states the fallback policy in place of the single-gateway caveat, names
this as a reversal of the 2026-07-28 ruling and why, and Section 6
(Probing etiquette) notes the per-host cap applies per gateway host.

---

## 2026-07-30 — SPEC DRIFT CHECK: no change (no semantic effect)

**What changed.** Nothing in the method. This entry records a *verification*,
not a revision — the pinned spec was re-checked against the standard rather
than assumed still current, and the check is logged whether or not it moves
anything so that "we checked" is itself falsifiable.

**Why.** Every rung-4 result carries a `spec_commit`, which is only meaningful
if someone confirms that commit still describes ERC-8004. The pin
(`68fc676`) dates from 2026-06-11 and had never been re-checked. A pinned
spec that has silently gone stale would mean the census judged a live
population against a superseded standard while continuing to cite it by
commit — the failure mode this check exists to rule out.

**Result.** Checked against **two independent sources**, because the repo the
copy was taken from is not the canonical home of the standard:

| source | result |
|---|---|
| `erc-8004/erc-8004-contracts` @ HEAD | HEAD **is** `68fc676` — unmoved since the pin |
| `ethereum/ERCs` @ `master`, `ERCS/erc-8004.md` | **byte-identical** |

All three files share the checksum `c92192bf60e67727ce87a99305ff9a31`.
**Zero normative differences, and in fact zero differences of any kind.**
The canonical text's last substantive change was **2026-01-25**
(`503591a6e80e`, "Updates from community feedback") — five months before the
pin. Status remains `Draft`.

**Measured effect.** **None.** No rung's rule changes, no agent's result
moves, no fixture is added, and no version is bumped: there is no semantic
change to record a before/after count against. The 354,858 results already
written were judged against text that is still current.

**Documented in** `spec/SOURCE.md` (new "Drift checks" section, with the
commands to repeat it) and `METHODOLOGY.md` §5.

---

## 2026-07-30 — VALIDATION REGISTRY: an assumption tested, and wrong

**What changed.** No rung's rule. What changed is a documented *assumption*
about coverage: `scripts/seed_chains.sql` asserted that the Validation
Registry is "absent on this chain", and `chains.validation_registry` is NULL
everywhere, which `crates/indexer` reads as absent and skips. The `validations`
table has zero rows. The census therefore described the last third of ERC-8004
as unused when what was true is that **it had never been looked at**.

**Why.** Those are different claims and only one was earned. The curated
upstream (`erc-8004/erc-8004-contracts`) publishes no Validation Registry
address for any of 30+ networks and says the contract is still under design —
which supports the assumption, but is a claim on a web page, so it was tested
against the chains instead of adopted.

**Method.** Scanning a known address would have inherited the assumption under
test — filtering on NULL returns zero, and zero would have been reported as
fact. Instead: scan the **spec's own event topics with no address filter**, so
any contract emitting an ERC-8004 validation event is found regardless of who
deployed it. `topic0` computed with `cast keccak` from the pinned spec's event
declarations (lines 362, 380).

**Measured effect.** No agent's rung result moves — no rung reads this
registry. The census gains a measurement it did not have:

| chain | agents | requests | responses | registries | agents validated |
|---|---:|---:|---:|---:|---:|
| base | 60,097 | 74 | 68 | 9 | 19 |
| bsc | 244,208 | 0 | 0 | 0 | 0 |
| mainnet | 40,806 | 0 | 0 | 0 | 0 |
| celo | 9,747 | 31 | 27 | 2 | 4 |
| **total** | **354,858** | **105** | **95** | **10** | **23** |

**23 of 354,858 agents — 0.0065%.** All ten deployments are third-party; nine
of ten answer `getIdentityRegistry()` with the canonical Identity Registry this
census sweeps, so they concern censused agents. The tenth (`0x279a126b…`, Celo)
**reverts** on that getter, which the pinned spec requires at line 347, so its
10 events are reported separately rather than pooled.

**Also measured, and previously unknown:** the Identity Registry's actual
deploy block on each chain — base 41,663,783, bsc 79,027,268, mainnet
24,339,871, celo 58,396,724 — by binary search on `eth_getCode`, verified on
both sides of the boundary. `seed_chains.sql` carried `deploy_block = 0` with a
comment asking for exactly these and warning against guessing a too-high value;
these are measurements, not guesses, and cannot be too high.

**Not applied to any database.** `seed_chains.sql` is updated in the
repository only. Running it is a deliberate act, and `DATABASE_URL` still
points at Supabase by default.

**Full detail** — including what may and may not be said about the 105
verdicts: `analysis/validation-registry.md`.

---

## 2026-07-30 — FEEDBACK VALUES: read for the first time, and a scope caveat Base needs

**What changed.** No rung's rule, and no agent's status. Two things the census
did not previously know:

1. **The feedback values themselves.** `chain::Reputation::feedback` calls
   `getSummary(agentId, clients, "", "")` and keeps `count`, discarding
   `summaryValue` and `summaryValueDecimals`. Rung 7 has always counted
   feedback without ever reading one.
2. **That Base's feedback figures describe a third of Base's feedback.**

**Why.** "29,570 agents carry feedback" says nothing about what the feedback
says. The open question was whether concentrated feedback is all one identical
value — the automation reading. It is.

**Method, and a departure from the brief, reported rather than resolved
silently.** The brief specified `getSummary`. This scans the `NewFeedback`
event instead: `getSummary` returns one aggregate per agent and so cannot show
the distribution *within* an agent, costs two RPC calls per attested agent
against a few hundred for the whole scan, and — decisively — **can only be
asked about agents the run knows about**, which is precisely what hid the
finding below. `topic0` computed with `cast keccak` and verified against a
known-good log (Base block 41,688,962) that matches a row the indexer had
independently stored.

**Validation.** Event-log counts versus the `feedback_count` rung 7 stored:

| chain | log entries | stored sum | |
|---|---:|---:|---|
| bsc | 29,507 | 29,507 | exact |
| celo | 27,532 | 27,532 | exact |
| mainnet | 3,209 | 3,209 | exact |
| base | 427,867 | 143,713 | 143,713 + 284,072 + 82 = 427,867 |

Per agent on Base, 29,568 of 29,571 agree exactly; the 82 are real
`FeedbackRevoked` events (which `getSummary` omits by design) affecting the
other 2 agents.

**Measured effect — the scope caveat.** The remaining 284,072 belong to
**agent 25975, which holds 66.4% of every feedback entry ever written on
Base and has no row in the run.** It is the single agent Base's manifest
already records as `unreadable` (`agent_count 60098, swept 60097,
unreadable 1`). The sweeper behaved correctly — a failed chain read is the
census's problem, not the agent's, so the agent is excluded rather than
failed, and the count is recorded durably. The failure was transient:
`ownerOf(25975)` answers at the pinned block and now. **What was never done
was joining that `1` to anything.**

> **No published number becomes wrong. "Feedback on Base" as an unqualified
> phrase becomes unsupportable.** Every Base feedback figure is correct for
> the 60,097 agents the run measured, and must now be labelled as covering
> **33.6% of the chain's feedback events**, naming the excluded agent.

BSC's 77 unreadable agents cost nothing — its log total matches its stored sum
exactly, so none of them holds any feedback. The Base gap is one agent that
happened to be the largest.

**The values.** 83.3% of Base's entries, and 66–78% elsewhere, carry their
tag's single most common value. The largest tag in the dataset,
`miner-vouch` (284,066 entries — agent 25975's traffic), has **exactly one
distinct value**. `trust` = 85 on all four chains (78.8–94.4%); `liveness` =
100 (95.8–99.3%). BSC's entire feedback population comes from **104
addresses**. The automation reading completes — with the caveat that an
automated prober writing the same value is behaving correctly, so this says
the layer is machine-written, not that it is dishonest.

**Full detail:** `analysis/feedback-values.md`.

---

## 2026-07-30 — RENAME: Ledgerscope becomes AgentCount (no semantic effect)

**What changed.** The project's name, domain and probe identity. Nothing about
what is measured.

- Product name **Ledgerscope → AgentCount**; domain **agentcount.ai**.
- The probe User-Agent becomes
  `agentcount-probe/0.2 (+https://agentcount.ai/methodology; contact: probes@agentcount.ai)`.
- Contact for suppression requests and disputes becomes
  **`probes@agentcount.ai`**.

**Why this is in the methodology changelog at all.** The User-Agent and the
contact address are not branding — they are **promises made to every host we
fetch from**, published in `METHODOLOGY.md` §6. An operator who saw our traffic
last week and wants to complain about it must be able to find us. Changing that
string is a change to a documented commitment, so it is recorded here even
though no rung moves.

**Measured effect.** **None.** No check's rule changes, no agent's status
moves, no schema or version bumps. The one behavioural consequence is that the
`User-agent:` token `robots.txt` is matched against changes from
`ledgerscope-probe` to `agentcount-probe`. A site that had specifically
targeted the old token — none is known to exist, the name was never
published — would stop matching. Wildcard `User-agent: *` rules, which is what
every observed `robots.txt` in the census actually uses, are unaffected.

**Fixture.** `crates/probe`'s User-Agent assertion
(`the_user_agent_is_assembled_from_the_product_token_and_contact_url`) and the
`robots.txt` token-matching test both carry the new string; all 50 probe tests
pass against it.

**Still blocked.** §6's standing note that a human must confirm
`probes@agentcount.ai` delivers **remains in force**. The domain is now real,
which the old note asked for; the mailbox is not yet confirmed, and rung 6 does
not ship until it is.
