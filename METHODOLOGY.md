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
- **Fail:** the URI is empty, malformed, unresolvable, times out, or returns
  any non-2xx status.
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

### Rung 7 — `independent`

**Question:** Has this agent received at least one Reputation Registry
feedback entry from an address that is not its own owner?

- **Pass:** at least one distinct feedback author differs from the current
  owner address.
- **Fail:** zero feedback entries exist, or every entry that exists was
  authored by the owner itself.
- **Evidence:** `feedback_count`, `distinct_authors`,
  `authors_equal_to_owner`, `self_feedback_ratio`.
- **Does not mean:** that the feedback is genuine, uncoordinated, or positive
  — only that it did not come from the agent's own owner address. Two
  addresses under common control, one feeding the other, would still pass
  this rung. Catching that would take clustering inference, which is a
  different kind of claim than a measurement — if we ever publish it, it goes
  in a separately-labelled `signals` block, never in a rung.

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

**Current implementation status, stated plainly:** at the time of this
Day-1 release, rung 1 (`registered`) is implemented and is the only rung a
sweep actually runs. Rungs 2 through 7 are fully specified above — their
pass/fail conditions and evidence shapes are final — but no code executes
them yet. Every agent in the current data has rows for rung 1 only; rungs 2–7
are absent for every agent, meaning exactly what absence means above: not yet
checked, not failed. This document describes the full ladder ahead of the
remaining six rungs shipping, on purpose — see the note at the top of this
file.

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
one assembled from reads taken minutes apart) reproduces the same rung-1
result set from the same code. If a `checker_commit` or `spec_commit` differs
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
to. Stated plainly first: **as of this writing, no component of this project
fetches an agent's document under the seven-rung model at all** — rungs 2 and
6 are specified (Section 2) but not yet implemented (Section 4). The commitments
below are what the probe layer landing next will implement; where an older,
soon-to-be-retired component in this codebase already behaves differently,
that's called out rather than glossed over.

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
- **What is not yet handled:** `robots.txt` is not yet checked or honored, and
  there is no per-host request cap independent of overall sweep concurrency.
  Both are planned for the probe layer; this document will be updated when
  they ship, not before.
- **Safety guard carried forward from the retiring enricher:** requests are
  only ever made to public IP addresses. An `agentURI` pointing at a private,
  loopback, link-local, or cloud-metadata address (`169.254.169.254` and
  similar) is refused before any connection is attempted, and redirects are
  not followed — so a registered agent cannot use a probe to reach, or bounce
  a request into, an internal network. This guard already exists in the
  current codebase and its replacement is expected to keep it.

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
