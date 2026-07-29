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
each agent is **seven questions and the evidence behind each one** —
`pass`, `fail`, `skipped`, `error`, or — for rung 5 alone, added 2026-07-29 —
`unclaimed`, per rung, per agent, per run. See §2's Rung 5 entry and §4 for
what each word means and why a fifth was needed.

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
- **`data:` URIs are decoded through five fallback paths, in order (P0 FIX
  7, 2026-07-29).** The ecosystem's convention is
  `data:application/json;enc=<algorithm>[;level=<n>];base64,<payload>` with
  zstd, gzip, brotli, or lz4 compression; production also uses several
  non-standard variants. The parser tries, in this order: (1) an `enc=`
  parameter — decompress with the named algorithm, then use the result; (2)
  any `;base64,` meta at all, regardless of declared MIME type or charset
  (`data:text/plain;base64,`, `data:;base64,`,
  `data:application/json;charset=utf-8;base64,` all decode identically); (3)
  no `;base64,` token — the payload is literal/percent-encoded text; (4) a
  payload that *claims* base64 but plainly starts with `{` or `[` — the
  decode is skipped and the payload used as-is (some real-world tooling
  forgets to actually encode); (5) no `data:` scheme at all — the on-chain
  URI string itself is raw JSON, treated the same as any other already-in-hand
  payload. Evidence always records which path succeeded
  (`data_uri_variant`) and, for a compressed payload, which algorithm
  (`data_uri_algorithm`) — which path succeeded is itself worth publishing.
  **Only `gzip` decompression is implemented.** Measured against the
  reference population (60,049 agents), every `enc=` occurrence (399 of
  them) declares `gzip`; zstd/brotli/lz4 — the ecosystem's *recommended*
  algorithm among them — have zero occurrences, so no dependency was added
  to decode them. **A `data:` URI declaring a compression algorithm this
  parser does not implement is this rung's `error`, not `fail`**: we
  understood exactly what was declared, we simply cannot decode it — that is
  our limitation, not a defect in the agent's document. See
  `CHANGELOG-METHODOLOGY.md`'s FIX-7 entry for the full measurement.
- **`ipfs://` is tried against up to three gateways, in sequence (P0 FIX 8,
  2026-07-29 — reverses a 2026-07-28 ruling).** The earlier ruling used one
  disclosed gateway (`ipfs.io`), specifically so a failure would be honestly
  attributable to either the agent or the gateway. The owner has reversed
  that ruling: a single gateway's outage was itself becoming indistinguishable
  from an agent's content being genuinely unpinned, which defeats the
  original goal. `https://ipfs.io/ipfs/`, `https://cloudflare-ipfs.com/ipfs/`,
  and `https://gateway.pinata.cloud/ipfs/` are now tried in that order; the
  first to answer HTTP 2xx wins. Evidence records every gateway attempted,
  each one's own status (`gateway_attempts`), and which one (if any) won
  (`via_gateway`) — the whole chain, not just the winner. **If all three
  gateways fail, this rung records `error`, never `fail`**: we cannot
  distinguish a CID no gateway has pinned from a problem on our end, and
  claiming otherwise would be a claim we cannot support. This affects the
  3,588 `ipfs://` agents in the reference population; see
  `CHANGELOG-METHODOLOGY.md`'s FIX-8 entry. The existing politeness rules
  (per-host concurrency cap, `robots.txt`, timeouts, the SSRF netguard on
  every hop) apply to each gateway host independently — three gateways are
  three hosts, not one host tried three times, so none of them can starve
  the others' budget or bypass the cap.
- **Evidence:** `uri` (from `tokenURI()`), `final_url` actually fetched,
  `http_status`, `elapsed_ms`, `fetched_at`, and, where applicable,
  `data_uri_variant`/`data_uri_algorithm` (inline `data:` documents) or
  `gateway_attempts`/`via_gateway` (`ipfs://` documents).
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

**Question:** Does the document violate anything the spec actually marks
MUST for the agent registration file? — with SHOULD and MAY gaps recorded
alongside, never collapsed into the same verdict.

**Since P0 FIX 3 (2026-07-29), rung 4 is not a single required-field
checklist.** The spec invokes RFC 2119/8174 explicitly (spec line 36):
MUST, SHOULD, and MAY are three different promises, and an earlier version
of this rung compressed all three into one `pass`/`fail` — the exact kind
of compression Section 1 of this document says Ledgerscope exists to
refuse, committed by Ledgerscope itself. Rung 4 now classifies every field
it looks at into one of the three severities and reports all three, always:

- **Pass:** zero MUST violations.
- **Fail:** one or more MUST violations.
- **Evidence:** `must_violations[]`, `should_gaps[]`, `may_gaps[]` — always
  all three arrays, on both `pass` and `fail`, plus `spec_commit` (the
  pinned spec commit this check was run against), `registrations_checked`
  (how many `registrations[]` entries were validated; 0 when the key is
  absent or empty), `services_field_source` / `legacy_endpoints_field` /
  `both_fields_present` (which of `services`/`endpoints` supplied the
  value — see below), and `services_status` (`"absent"` / `"empty"` /
  `"present"` — see below).
- **Does not mean:** that the field values are truthful, well-formed, or
  point anywhere real — only that the key exists in the document. A `name`
  field containing an empty string still counts as present; content quality
  is not this rung's question, at any severity.

**The uncomfortable finding, stated plainly.** Under an honest reading of
the pinned spec, the registration file has exactly **one MUST, and it is
conditional**: `registrations[].agentId` and `registrations[].agentRegistry`,
checked only when a `registrations` array is present at all (spec line 123,
"all fields in the registration are mandatory" — but the same sentence
downgrades the array's own presence to SHOULD). Everything this rung once
treated as unconditionally REQUIRED — `type`, `name`, `description`,
`image`, `services` — is SHOULD. `x402Support`, `active`, and
`supportedTrust` are MAY. This means the overwhelming majority of documents
that parse as JSON at all now pass rung 4: ERC-8004 imposes almost no hard
requirement on this document. **This is reported as measured, not
softened** — see `CHANGELOG-METHODOLOGY.md`'s FIX-3 entry for the exact
before/after population count. The interesting number moves to the
**SHOULD-completeness distribution** — how many of the seven SHOULD checks
a document actually satisfies — which this evidence shape makes queryable
for the first time without a re-sweep.

**The field list, and the rulings behind it.** The spec
([pinned at `68fc676`](spec/SOURCE.md)) never uses the literal word
"REQUIRED" for an agent-document field — it works entirely with "MUST",
"SHOULD", "MAY", "OPTIONAL", and, once, the plain-English synonym
"mandatory" (line 123). Every field's severity, and the reasoning behind
it — including two reversed rulings, kept in the record rather than edited
out — is in [`spec/REQUIRED_FIELDS.md`](spec/REQUIRED_FIELDS.md); the short
version:

- **MUST (2 fields, conditional):** `registrations[].agentId`,
  `registrations[].agentRegistry` — checked only when `registrations` is
  present as a non-empty array (line 123). This is the *only* thing that
  can fail rung 4.
- **SHOULD (7 checks):** `type`, `name`, `description`, `image` (line 115 —
  reverses a 2026-07-27 ruling that had held these REQUIRED; see
  `REQUIRED_FIELDS.md` Ruling 4 for why the earlier reading didn't survive
  being checked against the rest of the document's own severity pattern);
  `services`/`endpoints` (line 115's "endpoints" is the pinned spec's own
  legacy name for `services` — both accepted, `services` wins when both
  present, an empty array recorded distinctly from an absent key because
  the two describe different facts — a declared-but-unreachable agent vs.
  one that never engaged with this part of the schema); `registrations`
  itself, at least one entry (line 123); `services[].version` (line 115,
  aggregated to one document-level gap regardless of how many entries lack
  it).
- **MAY (3 fields):** `x402Support`, `active` (appear only in the spec's
  illustrative example JSON, lines 99-100, never in normative prose —
  originally miscounted as REQUIRED, corrected 2026-07-28); `supportedTrust`
  (line 124, explicitly marked OPTIONAL). A fourth field, `updatedAt`, is
  intentionally **not** checked at any severity — it does not appear
  anywhere in the pinned spec, despite surfacing in some downstream
  guidance; see `REQUIRED_FIELDS.md`'s MAY section for the full citation
  trail on why it was excluded rather than silently added.

A reader who takes any of the ambiguous sentences the other way will
disagree with some rung-4 verdicts. That disagreement is legitimate; it is
why every ruling is recorded — including the reversed one — rather than
discovered only from a `fail` or a surprising `pass`.

### Rung 5 — `bound`

**Question:** Does the off-chain document name the agent id, registry, and
chain it belongs to, and do they match the on-chain record it was actually
fetched *from*?

- **Pass:** the document's declared `registrations[].agentId` and
  `agentRegistry` (format `{namespace}:{chainId}:{identityRegistry}`) match
  the chain, registry address, and token id this fetch originated from.
- **Fail:** the document declares a registration — `registrations` is
  present with at least one entry — but no entry's agent id, registry, and
  chain all match the one we fetched it from.
- **Unclaimed** *(added 2026-07-29):* the document carries no `registrations`
  claim at all to check — the key is absent, or present as an empty array.
  **This is not a `fail`.** Once P0 FIX 3 made `registrations` a SHOULD
  rather than a MUST (see Rung 4), a document can pass rung 4 while making no
  binding claim whatsoever, and none of the other four statuses honestly
  describes that: `pass` would claim a verification that never happened,
  `fail` would punish a merely-recommended field as hard as a genuine
  mismatch, `skipped` would falsely imply an earlier rung failed, and `error`
  would falsely imply this checker malfunctioned. `unclaimed` is a
  publishable finding in its own right: how many agents pass conformance
  while declining to link their document back to their own on-chain
  identity. See `CHANGELOG-METHODOLOGY.md`'s 2026-07-29 rung-5-status entry
  for the measured population split.
- **Evidence:** `declared_agent_id`, `declared_registry`, `declared_chain`,
  `match` (`true`/`false` when a claim was checked, `null` when
  `unclaimed` — there was nothing to have matched or not), `reason`
  (`"unclaimed"`, present only on that status), `registrations_seen` (how
  many entries were evaluated; `0` for `unclaimed`).
- **Does not mean:** that the document is otherwise trustworthy — only that
  it is not, at minimum, a card copy-pasted wholesale from a different
  registration. This rung exists specifically to catch that pattern: a
  document that mentions the *wrong* registration is this rung's `fail`; a
  document that mentions none at all is `unclaimed`, not silently folded
  into either `pass` or `fail`.

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

### Rung 7 — `attested`

**Renamed from `independent` on 2026-07-29 (P0 FIX 4/5) — and ungated.**
Before this fix, rung 7 was called `independent` and asked whether at least
one feedback author differed from the agent's current owner; it also only
ran for the ~1,437 agents whose *document* had already passed rungs 1
through 5, on the mistaken assumption that it depended on the document
track at all. Both changed together, in the same run, because ungating this
rung without renaming it would have published the tautological finding
below at ~60,000-agent scale instead of ~1,437 — worse, not better.

**Question:** Has this agent received at least one Reputation Registry
feedback entry, from any client address at all?

- **Pass:** `getClients` returns at least one distinct address.
- **Fail, `no_feedback`:** `getClients` returns zero addresses — nobody has
  left this agent feedback.
- **Error, `no_reputation_registry`:** the chain has no Reputation Registry
  deployed at all. This is resolved once per chain, before the rung runs for
  any agent on it, and it is recorded as `error`, never `fail` — the agent
  did nothing wrong; the infrastructure to ask the question doesn't exist on
  that chain.
- **Gating:** rung 1 alone — **not** rungs 2 through 5. Reputation feedback
  lives in the Reputation Registry, keyed by agent id, and is readable
  regardless of whether that agent's document ever resolved, parsed,
  conformed, or bound to it; there was never a real dependency on the
  document track, only an accidental one from how the rung used to be gated.
  Rung 7 therefore runs for every agent that passes rung 1 — essentially the
  whole population — and its "not checked" count falls to zero except for
  agents where the chain read itself failed on our side.
- **How the data is read:** `getClients(agentId)` is called first, at the
  same pinned block every other rung's chain state is read at, to get the
  registry's own list of who has ever left this agent feedback. Only if that
  list is non-empty is `getSummary(agentId, clientAddresses, "", "")` called,
  passing the exact client list back in as the filter — `getSummary` with an
  empty `clientAddresses` array reverts on this contract rather than
  returning zero, so an empty `getClients` result short-circuits before
  `getSummary` is ever called. `feedback_count` comes from `getSummary`'s
  total entry count across the supplied clients; `distinct_authors` is
  computed from the `getClients` list itself, de-duplicated and compared
  case-insensitively.
- **Evidence:** `feedback_count`, `distinct_authors`, and `reason` (present
  only on `fail`/`error`: `no_feedback` or `no_reputation_registry`).
- **Does not mean:** that the feedback is genuine, uncoordinated, or positive
  — only that some exists. It does not read feedback *values*, and it makes
  no comparison against the agent's owner at all (see below) — two addresses
  under common control, one feeding the other, would still pass this rung.
  Catching that would take clustering inference, which is a different kind
  of claim than a measurement; if this project ever publishes it, it goes in
  a separately-labelled `signals` block, never in a rung, and it is **not
  built as of this fix**.

**Why this rung does not, and cannot, detect self-review.** The pinned spec
is explicit — `spec/ERC8004SPEC.md` line 217: *"The feedback submitter MUST
NOT be the agent owner or an approved operator for `agentId`."* Owner
self-feedback is a contract-level invariant, not a heuristic: it cannot be
successfully submitted in the first place. The old `independent` rung
compared feedback authors against the owner anyway and reported "zero agents
caught writing their own reviews" — which is not a measurement, since its
only possible outcome is restating a rule nobody can break. **No evidence
field, log line, or report this project produces claims to detect
self-review, because this rung does not check for it.** The two fields that
used to carry that comparison — `authors_equal_to_owner` and a derived
`self_feedback_ratio` — are gone from this rung's evidence entirely, not
renamed: keeping them would mean silently reintroducing the owner comparison
this fix removes, and empirically the comparison is nearly always the same
constant regardless (see `CHANGELOG-METHODOLOGY.md` for the one counter-
example found in the archived run, and why even that exception does not
rescue the metric — ownership can transfer after feedback was left, so
"current owner equals a past feedback author" is not evidence of self-review
either). `feedback_count` and `distinct_authors` survive unchanged: both are
genuine per-agent measurements a reader can recompute from
`getClients`/`getSummary` themselves, and neither implies anything about who
those authors are relative to the owner.

The spec itself flags the underlying problem it leaves unsolved (line 324):
`getSummary` requires a non-empty `clientAddresses` filter precisely because
unfiltered feedback is subject to Sybil/spam attacks, and it expects
reviewer-reputation to emerge off-chain — outside what this contract, or
this rung, can settle.

## 3. Ladder semantics

A rung that does not pass stops **everything that depends on it**: those
rungs are recorded as `skipped`, **never** as `fail`.

**Three independent tracks, not one chain (P0 FIX 4/5, 2026-07-29).** Before
this fix, "depends on" meant nothing more than "has a smaller rung number",
so any failure anywhere below rung 7 — even one rung 7 has nothing to do
with — silently skipped it. That was true only by accident of how rung 7
used to be gated, and it stopped being true the moment rung 7 was ungated to
run for every agent that passes rung 1. The dependency graph is now, and is
published as, three separate tracks:

- **Document** (1 → 2 → 3 → 4 → 5): a straight chain, each rung needs the one
  directly below it to have passed. Unchanged from before this fix.
- **Service** (6, not yet implemented): depends on rung 4 — a document that
  conforms enough to declare `services`, not the full chain back to rung 1,
  and *not* rung 5 (an agent can decline to bind its document to the chain
  and still have live endpoints worth checking).
- **Reputation** (7): depends on rung 1 *alone*. A rung-2, -3, -4, or -5
  failure must never skip rung 7 — reputation feedback is readable for any
  agent id that exists on chain, independent of whether its document ever
  resolved, parsed, conformed, or bound.

This is enforced in one place (`checks::run_ladder`) and is not a convenience
— it is the rule that keeps a failing lower rung from silently becoming
several failing higher ones **within its own track**, while never leaking
across tracks that have nothing to do with each other. If rung 2 fails
because the URI never resolved, we never received a document; rung 3 cannot
ask whether that document is valid JSON, because there is no document to
parse — but rung 7 can still be asked, and answered, because it never needed
that document in the first place. Recording `fail` for a question that was
never actually asked would misstate what happened, and is the single easiest
way to overstate a problem by accident. A `skipped` result carries which
rung stopped it and what that rung's status was (`skipped_because_rung`,
`skipped_because_status`), so nothing about the stoppage is lost — only the
dependent rungs' verdicts are withheld, because they could not be judged.

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
- **An absent rung** — no row exists for this (run, agent, rung) at all.
  **P0 FIX 6 (2026-07-29) narrows this to one meaning**: the rung has not
  been implemented yet (currently rung 6 alone). Before this fix, "this run
  did not attempt it" was a second, silently overlapping reason an
  implemented rung could also be absent — rungs 4 and 5 were only
  *constructed* when a document existed to judge, so an agent whose document
  never resolved or never parsed got no rung-4/5 row at all, indistinguishable
  from rung 6, which genuinely is not implemented. That case is `skipped`
  now, not absent — see `CHANGELOG-METHODOLOGY.md`'s FIX 6 entry for the
  defect and its measured population effect. The one remaining exception is
  an agent absent from the run *entirely* (every rung, not just one) because
  its chain read or database write failed on our side — reported separately
  by `crates/sweeper` as `unreadable`/`unwritable`, never presented as a
  per-rung gap. **An absent rung is never a claim that the agent failed it.**
  The schema enforces this at the storage layer: a row is written only when a
  check actually ran, so there is no default value to accidentally read as a
  verdict (no `COALESCE(x, false)` anywhere in this pipeline). If you do not
  see a rung for an agent, the honest reading is "not yet checked," full
  stop — not "presumed to fail."
- **`unclaimed`** *(rung 5 only, added 2026-07-29)* — not one of the three
  kinds above, and deliberately its own word: the rung was asked, it ran to
  completion, and it found nothing to check, because the agent made no
  binding claim (no `registrations` array, or an empty one) for it to verify.
  It is not a consequence of a lower rung failing (`skipped`), not an absence
  of any row (a rung-5 row always exists for an agent that reached rung 5),
  and not a checker malfunction (`error`) — see Rung 5's entry in §2 for the
  full reasoning.

**Current implementation status, stated plainly:** rungs 1 (`registered`),
2 (`resolvable`), 3 (`parseable`), 4 (`conformant`), 5 (`bound`), and 7
(`attested`, renamed from `independent` 2026-07-29) are implemented and run
in every sweep. Rung 7 runs for every agent that passes rung 1 — essentially
the whole population, not the small rung-1-through-5 intersection it used to
be gated on; the only agents it produces no row for are the rare case where
rung 1 itself fails (an owner-is-zero-address token) and the read is never
attempted, plus any agent where the chain read failed on our side (counted
and reported, never silently dropped — see `crates/sweeper`). **Rung 6
(`live`) is not implemented** — it is fully specified above, its pass/fail
conditions and evidence shape are final, but no code executes it, and it
writes no row for any agent. Rungs 4 (`conformant`) and 5 (`bound`) are
**always constructed**, for every swept agent, regardless of whether a
document ever existed to judge (P0 FIX 6, 2026-07-29) — when there is
nothing to judge, they come back `skipped`, naming whichever earlier rung
in the Document track actually blocked them, never absent. A run's data
therefore has, for every agent, rows for rungs 1, 2, 3, 4, 5, and (for
nearly everyone) 7, and no row at all for rung 6 — meaning exactly what
this section says an absent rung means: not yet checked, not failed. This
document described the full ladder ahead of rungs 2–7 shipping; that has
since happened for all but rung 6, and this paragraph is updated each time
the implemented set changes rather than left to describe a state that no
longer holds. **This guarantee holds for runs swept with checker code that
includes FIX 6 or later; the archived reference run
(`1c87c4f4-c4c4-45ee-b03a-d8517f4d5d8a`, finished 2026-07-28) predates the
fix and does not have it** — see the FIX 6 changelog entry for exactly which
rows in that run are affected and why the run is not rewritten to match.

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
  This applies per **host**, so the three-gateway IPFS fallback chain
  (Section 2, Rung 2, P0 FIX 8) counts as three separate hosts, each
  under its own cap — not one host budget shared, or bypassed, across all
  three.
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
