# Methodology

This document is published before any findings exist. It describes what
Ledgerscope measures and how, so that anyone reading a result later — a
registrant, a journalist, a researcher — can check the method before checking
the number, not after.

If a future finding and this document ever disagree, this document is wrong
and should be corrected; a methodology edited to fit its results has stopped
being one.

## 1. What this is

Ledgerscope is an independent conformance and census layer for
[ERC-8004](spec/SOURCE.md) ("Trustless Agents"). It enumerates every agent
registered in an Identity Registry, reads the chain's current state for each
one, fetches and evaluates the off-chain document it points at, and checks
whether its declared endpoints and reputation entries hold up. The result for
each agent is **seven booleans and the evidence behind each one** —
`pass`, `fail`, `skipped`, or `error`, per rung, per agent, per run.

There is no aggregate. Not a score, not a grade, not a tier, not a ranking.
This is deliberate, not an oversight:

- **A 0–100 number needs weights**, and weights need something to calibrate
  them against — a known-good set of agents, a known-bad set, an outcome to
  predict. No such ground truth exists for this population. Any weighting we
  chose would be a design decision dressed up as a measurement, and a reader
  has no way to tell the difference between a calibrated score and an
  aesthetic one. So we don't produce one.
- **A boolean is falsifiable; a score is not.** "This document is missing the
  `services` field" can be checked by anyone with the URL and five minutes. A
  compressed 61 can only be checked by trusting our arithmetic.
- **Population base rates are still published**, and are in fact the point:
  "N of M agents that pass rung 1 also pass rung 4" is a fact about the
  population, derived by counting, not by weighting. What is never published
  is a single number that stands in for one agent.

Every claim in this document and everything it produces carries evidence a
reader can go re-check: a transaction hash, an HTTP response, a JSON diff, a
block number. Where we could not check something, we say so instead of
guessing.

## 2. The seven rungs

The ladder is ordered: each rung asks a question that only makes sense once
the rung below it has passed. A rung is judged only against the agent's
current on-chain state and the document it currently resolves to — not
against history, and not against reputation with any third party.

### Rung 1 — `registered`

**Question:** Does this agent id exist in the Identity Registry, as a
currently-held ERC-721 token?

- **Pass:** the registry returns a token for this id and `ownerOf()` resolves
  to a non-zero address.
- **Fail:** the token exists but `ownerOf()` returns the zero address (burned
  or otherwise unheld).
- **Evidence:** `chain_id`, `registry` (contract address), `token_id`,
  `owner`, `block_number` the read was pinned to, and `tx_hash` when the
  registration transaction is known (`null`, never an invented value, when it
  is not).
- **Does not mean:** that anyone controls the address that owns the token, or
  that the agent has ever done anything beyond mint. This is the floor every
  other rung is measured against — the population denominator, not an
  accomplishment.

### Rung 2 — `resolvable`

**Question:** Does `tokenURI()` return a URI, and does fetching it return a
successful HTTP response within the timeout?

- **Pass:** the URI dereferences (`https://`, `data:`, or `ipfs://` via a
  gateway) and, for a network fetch, the origin responds with HTTP **2xx**
  inside the configured timeout.
- **Fail:** the URI is empty or malformed, or returns any non-2xx status.
  Also **fail**, not error: a hostname that fails DNS resolution, or that
  resolves only to a private, loopback, or link-local address (the classic
  SSRF targets, `169.254.169.254` and similar). An agent-published URI that
  no third party can retrieve — because it does not resolve, or resolves
  only to an address nobody outside its own network can reach — is a fact
  about that document, the same category as an empty URI. Our SSRF guard is
  why *we* never attempted the request, but the reason it was unattemptable
  belongs to the agent, not to us; the evidence carries the one-line reason
  (`ssrf_blocked: dns resolution failed`, `ssrf_blocked: resolves to a
  non-public address`) so this is checkable, not asserted.
- **`robots.txt` is honored, redirects included.** Before fetching the
  document itself, we fetch and honor `/robots.txt` for that origin. A
  redirect on `robots.txt` (`http`→`https`, `www`→apex, and the like) is
  completely ordinary, so we follow up to **5** redirect hops — per RFC 9309
  §2.3.1.2, which asks clients to follow at least five — re-validating the
  SSRF guard on every hop the same way a redirect on the document itself is
  validated. A 2xx `robots.txt` is parsed and applied; a 4xx means no
  restriction and we proceed; a 5xx, a timeout, a connection failure, or a
  redirect chain that loops or runs past 5 hops means we could not establish
  permission — that is **our** limitation, so it is recorded as this rung's
  `error`, never as the agent's `fail`.
- **HTTP 402 fails this rung.** This is a deliberate ruling, not an
  oversight, and reverses an earlier design of this project that treated 402
  as "alive." A 402 response is a payment challenge, not the agent
  registration document — we have not received the document, so we cannot
  judge whether it resolves, and marking that a pass would silently promote
  "asked me to pay" into "gave me the file." The HTTP status and whatever
  body accompanied it are archived verbatim regardless of the verdict; x402
  support may return later as its own labelled signal, but it is never a
  substitute for this rung passing.
- **Evidence:** `uri` (from `tokenURI()`), `final_url` actually fetched,
  `http_status`, `elapsed_ms`, `fetched_at`.
- **Does not mean:** that the response body is the agent document, is JSON,
  or contains anything meaningful — only that something answered. That is
  rung 3's question.

### Rung 3 — `parseable`

**Question:** Does the fetched body parse as valid JSON?

- **Pass:** the bytes returned at rung 2 decode as a syntactically valid JSON
  value.
- **Fail:** the bytes are present but do not parse (truncated, HTML error
  page, binary, malformed JSON).
- **Evidence:** `content_type` header, `body_sha256`, `body_bytes` (length),
  `parse_error` (the parser's message, when parsing failed).
- **Does not mean:** that the JSON has the shape of an agent registration
  file, or that any field it needs is present. A valid empty object `{}`
  passes this rung and fails the next one.

### Rung 4 — `conformant`

**Question:** Does the document contain every field the spec marks REQUIRED
for the agent registration file?

- **Pass:** every field in the checklist below is present.
- **Fail:** one or more is missing.
- **Evidence:** `fields_found[]`, `fields_missing[]`, `spec_commit` (the
  pinned spec commit this check was run against), `registrations_checked`
  (how many `registrations[]` entries were validated; 0 when the key is
  absent). *Naming note: an earlier draft of this document called this
  evidence field `spec_version`. The implementation uses `spec_commit`
  instead, matching the field the run already carries elsewhere in this
  document (see the run-provenance table below) — a SHA is what makes the
  claim checkable, and one name for one thing avoids the two fields drifting
  apart.*
- **Does not mean:** that the field values are truthful, well-formed, or
  point anywhere real — only that the key exists in the document. A `name`
  field containing an empty string still counts as present; content quality
  is not this rung's question.

**The field list, and the ruling behind it.** The spec
([pinned at `68fc676`](spec/SOURCE.md)) never uses the literal word
"REQUIRED" for an agent-document field — it establishes required structure
with "The registration file **MUST** have the following structure" (spec line
54) and, once, the word "mandatory" (line 123). Two places in the spec text
are genuinely ambiguous about what that MUST governs, and both were decided
by the project owner rather than left to inference. Both rulings, and the
full extraction with every field's source line, are recorded in
[`spec/REQUIRED_FIELDS.md`](spec/REQUIRED_FIELDS.md); the short version:

- **7 fields checked unconditionally, on every document:** `type`, `name`,
  `description`, `image`, `services`, `x402Support`, `active`. (Ruling 1: a
  later sentence saying these four fields "SHOULD ensure compatibility with
  ERC-721 apps" was read as constraining their *content*, not downgrading
  their *presence* — line 54's MUST still governs whether they appear.)
- **2 fields checked conditionally, only when a `registrations` array is
  present in the document:** `registrations[].agentId` and
  `registrations[].agentRegistry`. (Ruling 2: the spec says agents "SHOULD
  have at least one registration," which downgrades the *array's own
  presence* to optional — a document with no `registrations` key does not
  fail rung 4 on that basis — but the same sentence also says "all fields in
  the registration are mandatory," so any entry that does exist must carry
  both sub-fields.)

A reader who takes either ambiguous sentence the other way will disagree with
some rung-4 verdicts. That disagreement is legitimate; it is why the ruling
is recorded here rather than discovered only from a `fail`.

### Rung 5 — `bound`

**Question:** Does the off-chain document name the agent id, registry, and
chain it belongs to, and do they match the on-chain record it was actually
fetched *from*?

- **Pass:** the document's declared `registrations[].agentId` and
  `agentRegistry` (format `{namespace}:{chainId}:{identityRegistry}`) match
  the chain, registry address, and token id this fetch originated from.
- **Fail:** the document declares a different agent id, registry, or chain
  than the one we fetched it from.
- **Evidence:** `declared_agent_id`, `declared_registry`, `declared_chain`,
  `match` (boolean per field compared).
- **Does not mean:** that the document is otherwise trustworthy — only that
  it is not, at minimum, a card copy-pasted wholesale from a different
  registration. This rung exists specifically to catch that pattern: a
  document that never mentions any registration is a rung-4 question (the
  `registrations` array is conditionally required, see Rung 4), but a
  document that mentions the *wrong* one is this rung's question.

### Rung 6 — `live`

**Question:** Does every service endpoint the document declares respond to a
HEAD or GET request?

- **Pass:** every declared endpoint returns HTTP 2xx.
- **Fail:** at least one declared endpoint does not.
- **Evidence:** a per-endpoint record of `url`, `status`, `elapsed_ms`,
  `checked_at`.
- **Does not mean:** that the endpoint does anything useful, correctly, or at
  all beyond answering the HTTP request — only that something is listening
  and answering. A single unreachable endpoint among several still fails the
  whole rung; a document that declares zero endpoints is a rung-4 concern
  (`services` presence), not this one.
- **Not yet implemented.** The pass/fail conditions and evidence shape above
  are final, but no code executes this rung yet — `crates/checks` has no
  `rung6` module, and `checks::run_ladder` never calls one. **This rung
  produces no row for any agent in any run to date.** An absent rung 6 means
  exactly what Section 4 says an absent rung always means: we did not ask,
  not that every agent's endpoints failed to answer. Do not read a rung-6
  gap in a report as a rung-6 failure — it is not in the data at all.

### Rung 7 — `independent`

**Question:** Has this agent received at least one Reputation Registry
feedback entry from an address that is not its own owner?

- **Pass:** at least one distinct feedback author (from `getClients`) differs
  from the current owner address (from `ownerOf`, the same snapshot every
  other rung uses).
- **Fail, `no_feedback`:** `getClients` returns zero addresses — nobody, self
  or otherwise, has left this agent feedback.
- **Fail, `only_self_feedback`:** `getClients` returns one or more addresses,
  and every one of them equals the owner address.
- **Error, `no_reputation_registry`:** the chain has no Reputation Registry
  deployed at all. This is resolved once per chain, before the rung runs for
  any agent on it, and it is recorded as `error`, never `fail` — the agent
  did nothing wrong; the infrastructure to ask the question doesn't exist on
  that chain. Precedence is absolute: even if the input happens to contain a
  non-owner client address, an unavailable registry still errors rather than
  passing, because a read that couldn't have happened isn't trustworthy
  evidence either way.
- **How the data is read:** `getClients(agentId)` is called first, at the
  same pinned block every other rung's chain state is read at, to get the
  registry's own list of who has ever left this agent feedback. Only if that
  list is non-empty is `getSummary(agentId, clientAddresses, "", "")` called,
  passing the exact client list back in as the filter — `getSummary` with an
  empty `clientAddresses` array reverts on this contract rather than
  returning zero, so an empty `getClients` result short-circuits before
  `getSummary` is ever called. `feedback_count` comes from `getSummary`'s
  total entry count across the supplied clients; `distinct_authors` and
  `authors_equal_to_owner` are computed from the `getClients` list itself,
  de-duplicated and compared case-insensitively so that address casing never
  affects the verdict.
- **Evidence:** `feedback_count`, `distinct_authors`,
  `authors_equal_to_owner`, `self_feedback_ratio`, and `reason` (present only
  on `fail`/`error`: `no_feedback`, `only_self_feedback`, or
  `no_reputation_registry`).
- **Does not mean:** that the feedback is genuine, uncoordinated, or positive
  — only that it did not come from the agent's own owner address. Two
  addresses under common control, one feeding the other, would still pass
  this rung. Catching that would take clustering inference, which is a
  different kind of claim than a measurement — if we ever publish it, it goes
  in a separately-labelled `signals` block, never in a rung.

**`self_feedback_ratio` needs its own paragraph, because a ratio in a
product that just spent Section 1 explaining why it publishes no score
invites exactly one question: isn't this a score in disguise? It is not,
for the same reason `body_bytes` in rung 3 or `registrations_checked` in
rung 4 aren't: it is per-rung evidence describing **one measurement** —
what fraction of *this agent's* distinct feedback authors is the owner —
not a number that combines several rungs, or several agents, into a
verdict. It is defined as `authors_equal_to_owner / distinct_authors`, and a
reader can recompute it from those two other evidence fields without
trusting our arithmetic, the same falsifiability bar every other field in
this document is held to. Nothing about it ranks agents against each other,
weighs anything, or produces a single figure meant to stand in for "how
trustworthy is this agent" — it stays scoped to the one rung, the one
agent, the one measurement it describes.

It is **`null`, never `0.0`, when `distinct_authors` is `0`.** A measured
zero and an unmeasurable quantity are different claims: "every author we
found happens not to be the owner" (a zero we actually observed) is not the
same statement as "there were no authors to check in the first place" (a
ratio with nothing to divide). Writing `0.0` for the second case would read,
to anyone who didn't check `distinct_authors` too, as the friendlier of the
two claims when it is actually the absence of any claim at all — exactly
the kind of silent wrongness Section 1 says this project exists to avoid.
`no_feedback` agents (`distinct_authors: 0`) always carry a `null` ratio;
only agents with at least one client ever carry a numeric one.

## 3. Ladder semantics

The rungs are evaluated in order, and a rung that does not pass stops the
ladder: everything above it is recorded as `skipped`, **never** as `fail`.

This is enforced in one place (`checks::run_ladder`) and is not a convenience
— it is the rule that keeps a failing lower rung from silently becoming
several failing higher ones. If rung 2 fails because the URI never resolved,
we never received a document; rung 3 cannot ask whether that document is
valid JSON, because there is no document to parse. Recording `fail` for a
question that was never actually asked would misstate what happened, and is
the single easiest way to overstate a problem by accident. A `skipped` result
carries which rung stopped it and what that rung's status was
(`skipped_because_rung`, `skipped_because_status`), so nothing about the
stoppage is lost — only the higher rungs' verdicts are withheld, because they
could not be judged.

## 4. What `skipped`, `error`, and an absent rung each mean

Three different kinds of "we don't have a `pass`/`fail` for this rung," and
they are not interchangeable:

- **`skipped`** — a lower rung did not pass, so this question could not be
  meaningfully asked. Carries evidence naming which rung stopped it. Not a
  claim about the agent at this rung; a consequence of a claim already made
  at a lower one.
- **`error`** — the check could not be completed and the fault is **ours**:
  an RPC call failed, our prober timed out for reasons unrelated to the
  target (e.g., our own network outage), a bug threw. `error` also stops the
  ladder exactly like a `fail` would (everything above it becomes `skipped`),
  but it is recorded distinctly so a reader can tell "the agent failed this"
  from "we failed to find out."
- **An absent rung** — no row exists for this (run, agent, rung) at all,
  because that rung has not been implemented yet, or this run did not attempt
  it. **An absent rung is never a claim that the agent failed it.** The
  schema enforces this at the storage layer: a row is written only when a
  check actually ran, so there is no default value to accidentally read as a
  verdict (no `COALESCE(x, false)` anywhere in this pipeline). If you do not
  see a rung for an agent, the honest reading is "not yet checked," full
  stop — not "presumed to fail."

**Current implementation status, stated plainly:** rungs 1 (`registered`),
2 (`resolvable`), 3 (`parseable`), 4 (`conformant`), 5 (`bound`), and 7
(`independent`) are implemented and run in every sweep. **Rung 6 (`live`) is
not implemented** — it is fully specified above, its pass/fail conditions
and evidence shape are final, but no code executes it, and it writes no row
for any agent. A run's data therefore has, for every agent, rows for rungs
1, 2, 3, 4, 5, and 7, and no row at all for rung 6 — meaning exactly what
Section 4 says an absent rung means: not yet checked, not failed. This
document described the full ladder ahead of rungs 2–7 shipping; that has
since happened for all but rung 6, and this paragraph is updated each time
the implemented set changes rather than left to describe a state that no
longer holds.

## 5. How to recompute any result

Every result is meant to be reproduced by someone who is not us. Each sweep
("run") is immutable once written and stamps four pieces of provenance onto
every row it produces:

| Field | What it pins |
|---|---|
| `schema_version` | The shape of `check_results.evidence` this run wrote against — bumped when the evidence contract changes. |
| `checker_version` | The `checks` crate's own version (its semantics — what "pass" means for each rung). |
| `checker_commit` | The exact git commit of the code that ran, stamped at build time from `git rev-parse HEAD`. A build from an uncommitted tree is stamped `<sha>-dirty` rather than silently claiming a clean commit describes it. |
| `spec_commit` | The `ERC8004SPEC.md` commit ([pinned in `spec/SOURCE.md`](spec/SOURCE.md)) rung 4 was judged against. |

Every run also records a literal `rerun_command` — for example:

```
cargo run -p sweeper -- base   # at block 41817815
```

Re-running that command against the same chain state (the pinned block makes
this exact — every agent's owner and URI are read at one block, so the
population a run measures is the population that existed simultaneously, not
one assembled from reads taken minutes apart) reproduces the same result set
— across every rung the sweep implements at that commit — from the same
code. If a `checker_commit` or `spec_commit` differs
between two runs, that is expected: the code or the spec changed. What must
never differ is a run's ability to point at the exact commit and spec version
that produced it — a result that cannot name what generated it is an
assertion, not a fact.

The full run — its manifest and one JSON document per agent, carrying its own
`checker_commit`/`spec_commit` so a single agent file is self-describing even
handed to someone without the rest of the run — is exported to
`data/<run_id>/` alongside the Postgres rows, specifically so it can be
downloaded and diffed without a database connection.

## 6. Probing etiquette

Rungs 2 and 6 fetch resources we do not control: an agent's declared document
and its declared service endpoints. This is the policy that behavior commits
to. The probe layer described below (`crates/probe`) now implements rung 2's
fetching, robots.txt handling, and redirect-following; rung 6 (checking each
declared service endpoint) is still specified (Section 2) but not yet
implemented. Where an older, retired component in this codebase used to
behave differently, that's called out rather than glossed over.

- **User-Agent:** every request will identify itself, e.g.
  `ledgerscope-probe/0.2 (+https://ledgerscope.io/methodology; contact: probes@ledgerscope.io)`
  — never disguised as a browser. A predecessor component in this codebase
  (`crates/enricher`, being retired) currently sends a similar but
  unreachable-by-design User-Agent
  (`ledgerscope-observer/0.1 (+https://ledgerscope.example/methodology)`,
  `.example` being a reserved TLD that can never resolve). This document
  exists in part to fix that before its replacement ships.
- **Contact:** `probes@ledgerscope.io`. If our traffic is a problem for you —
  volume, timing, anything — email that address with the agent id or host in
  question and we will suppress it from future sweeps. There is no automated
  self-serve opt-out; exclusion will be manual and best-effort on our side,
  stated plainly rather than promising infrastructure that does not exist.
- **Request rate:** the intent is one fetch per agent per rung per run, not
  continuous polling, with bounded concurrency and both a connect and an
  overall timeout per request so one slow or hanging endpoint cannot stall a
  whole sweep. Exact numbers (concurrency limit, timeout durations) belong to
  the probe layer's implementation, not to this document, so they can change
  without this policy going stale — check the probe layer's own
  configuration for the current values in force.
- **Methods used:** HTTP GET for rung 2 (fetching the registration document);
  HEAD, falling back to GET, for rung 6 (checking each declared service
  endpoint). Probing does not crawl beyond the single declared document and
  the service endpoints it lists — nothing here is meant to spider a site.
- **`robots.txt` is checked and honored**, including its own redirects: a
  301/302 on `/robots.txt` (`http`→`https`, `www`→apex, and the like) is
  followed for up to 5 hops, per RFC 9309 §2.3.1.2, before the 2xx/4xx/5xx
  mapping in Rung 2 is applied to whatever response is left standing. There
  is also a per-host request cap (independent of, and tighter than, overall
  sweep concurrency) so one host never sees more than a couple of our
  requests in flight at once, no matter how busy the rest of the sweep is.
- **Safety guard carried forward from the retiring enricher:** requests are
  only ever made to public IP addresses. An `agentURI` pointing at a private,
  loopback, link-local, or cloud-metadata address (`169.254.169.254` and
  similar) is refused before any connection is attempted. Redirects on the
  document itself, and on `robots.txt`, ARE followed — disabling them
  outright made an ordinary redirect (behind which a large share of the
  registry sits) indistinguishable from a real failure — but every hop is
  re-validated against this same guard before it is fetched, exactly like
  the first request, so a registered agent cannot use a redirect to bounce
  a request into an internal network.

> **This URL needs human confirmation before any probing goes live.**
> `ledgerscope.io` and the `probes@` mailbox are the proposed values recorded
> here so the policy has a citable home; a human must register the domain (or
> substitute a real one already owned) and confirm the mailbox actually
> delivers before any prober ships this User-Agent. Shipping the current
> placeholder (`ledgerscope.example`, a reserved TLD that can never resolve)
> would leave an operator who wants to complain about our traffic with
> nowhere to go — which is the problem this document exists to fix.

## 7. Limitations, stated plainly

This section exists because a passing or failing rung is easy to
misread, and the misreading is usually unfair to whoever registered the
agent. If you are reading this because your agent failed a rung, this is for
you.

- **A passing ladder is not an endorsement.** Seven passes mean seven
  specific, narrow questions were answered affirmatively at one moment. They
  are not a claim that the agent is safe, competent, well-run, or worth
  trusting with anything. Do not cite a full pass as a certification; it
  isn't one, and we do not intend it to be read as one.
- **A failing rung is a dated observation about a document, not a claim about
  a person or a project.** "This document was missing `services` on
  2026-07-26 at 14:03 UTC" is what we measured. It is not "this team is
  careless," "this project is abandoned," or any characterization of the
  people behind it. Documents get fixed; a fail today says nothing about
  tomorrow, and we don't say it does.
- **We observe from one network location, at one moment.** A DNS
  misconfiguration visible from our vantage point, or an endpoint mid-deploy
  when we happened to fetch it, reads identically to a genuine problem in our
  data. We do not have a second vantage point to cross-check against yet, and
  we say so rather than implying our single view is authoritative.
- **A clean ladder is not absolution.** Seven passes mean the seven questions
  above were answered, and nothing more. We do not currently publish any
  coordination or clustering analysis; if we ever do, it will appear in a
  separately-labelled `signals` block and will never be presented as a rung,
  because a heuristic and a measurement are different kinds of claim and
  deserve different amounts of your trust. Either way, the absence of an
  adverse finding means no pattern we look for was found in the data we have
  — not that there is nothing to find.
- **We measure what the chain and the document say, not what is true.** An
  owner address, a declared endpoint, a feedback author — all of these are
  what the on-chain and off-chain records state. Nothing in this pipeline
  verifies the real-world identity behind an address or a domain.
- **The ladder can change.** Rung definitions, the required-field list, and
  evidence shapes are versioned (`checker_version`, `spec_commit`,
  `schema_version`) precisely because they are expected to be revised as the
  ERC-8004 spec itself moves past Draft status and as we learn our checks
  were wrong. A revision is a correction, not evidence that earlier results
  were dishonest — they were accurate to the method in force when they were
  produced, and that method is recorded alongside them.

If you believe a specific result is wrong under the rules stated above —
not "unfair" in the abstract, but factually incorrect given the evidence
attached to it — email `probes@ledgerscope.io` with the run id and agent id
and we will look at it.
