# Methodology

This document is published before any findings exist. It describes what
AgentCount measures and how, so that anyone reading a result later — a
registrant, a journalist, a researcher — can check the method before checking
the number, not after.

If a future finding and this document ever disagree, this document is wrong
and should be corrected; a methodology edited to fit its results has stopped
being one.

## 1. What this is

AgentCount is an independent audit layer for the agent economy, built as
instruments that each enumerate a population and ask it checkable yes/no
questions. Sections 2–9 describe the first instrument; §10 the second.

The first instrument is a conformance and census layer for
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
- **Fail:** the URI is empty or malformed, or returns a non-2xx status meaning
  the document is not being served — 400, 403, 404, 410, 500, 502, 504 and the
  like. Also **fail**, not error: a hostname that fails DNS resolution, or that
  resolves only to a private, loopback, or link-local address (the classic
  SSRF targets, `169.254.169.254` and similar). An agent-published URI that
  no third party can retrieve — because it does not resolve, or resolves
  only to an address nobody outside its own network can reach — is a fact
  about that document, the same category as an empty URI. Our SSRF guard is
  why *we* never attempted the request, but the reason it was unattemptable
  belongs to the agent, not to us; the evidence carries the one-line reason
  (`ssrf_blocked: dns resolution failed`, `ssrf_blocked: resolves to a
  non-public address`) so this is checkable, not asserted.
- **`refused` (added 2026-08-06): the origin is there and declined us.** Five
  HTTP statuses, in the two groups the HTTP specification itself separates:
  **429 and 503**, the statuses defined to carry `Retry-After` and to mean "not
  now", and **401, 402 and 407**, the statuses that answer with a challenge
  rather than an absence. Plus the two `robots.txt` outcomes below. None of
  these tells us anything about the document, so none of them can honestly be
  the agent's `fail` — and none of them is a malfunction of ours, so none can
  be `error`. It is **not a pass**: we did not receive the document, and every
  rung above this one is `skipped` exactly as it would have been before.
  Read the 2026-08-06 changelog entry before quoting any pre-2026-08-06 fail
  or error rate against a later one.
  - **403 is not `refused`, and neither are 500/502/504.** A 403 refuses
    without offering a way in, which is indistinguishable from "this file is
    not available to third parties"; a broken upstream means the document
    really is not being served. Both are `fail`.
  - **HTTP 402 is `refused` and still does not pass.** The 2026-07-28 ruling —
    that a payment challenge is not the registration document, and that marking
    it a pass would promote "asked me to pay" into "gave me the file" — is
    unchanged and unaffected. Only the word moved, from the agent's `fail` to
    the more precise "something answered and asked for money", which is the
    same shape as a 401. Its evidence keeps the distinct reason
    `payment_required`, so paywalled documents stay countable on their own.
    x402 support may still return as its own labelled signal; it is never a
    substitute for this rung passing.
- **`robots.txt` is honored, redirects included — and honoring it is
  `refused`, not `error` (2026-08-06).** Before fetching the document itself,
  we fetch and honor `/robots.txt` for that origin. A redirect on `robots.txt`
  (`http`→`https`, `www`→apex, and the like) is completely ordinary, so we
  follow up to **5** redirect hops — per RFC 9309 §2.3.1.2, which asks clients
  to follow at least five — re-validating the SSRF guard on every hop the same
  way a redirect on the document itself is validated. A 2xx `robots.txt` is
  parsed and applied; a 4xx means no restriction and we proceed; a 5xx, a
  timeout, a connection failure, a body that is not UTF-8, or a redirect chain
  that loops or runs past 5 hops means we could not establish permission, and
  **we send no request** (RFC 9309 §2.3.1.4's conservative reading — see §6 for
  the full policy and why it was not loosened). Both outcomes — an explicit
  `Disallow` and an unavailable `robots.txt` — are recorded as this rung's
  `refused`, with the reason verbatim, never as the agent's `fail` and no
  longer as our `error`. They were `error` until 2026-08-06, which made the
  published error rate a measure of one host's `/robots.txt`: on the 2026-08
  mainnet run it read **22.1%**, of which 6,133 agents were a single host
  refusing connections on that one path.
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
- **"Inline" is an evidence field, never a test on the URI text.** The
  population of inline documents is `scheme == "data"` in this rung's
  evidence. It is *not* the agents whose `agent_uri` begins with `data:`, and
  the difference is large enough to change a published finding. Across the
  four-chain census the evidence field counts **182,775** inline documents and
  the string test counts **179,647** — a gap of 3,128, made of two things:
  **+3,151** agents whose `tokenURI()` is a bare JSON document with no scheme
  at all (decode path 5 above), which are inline by every meaning that
  matters — nothing was fetched — and **−23** agents whose URI does begin
  `data:` but is malformed (`;base64` followed by a space instead of a comma),
  which this rung records as `unsupported` because nothing was decoded from
  them. A report that splits agents into "inline" and "fetched over the
  network" must read the evidence, or it will describe 3,128 agents as running
  a server that was never contacted.
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
  `data_uri_variant`/`data_uri_algorithm` (inline `data:` documents),
  `gateway_attempts`/`via_gateway` (`ipfs://` documents), or `retry_after`
  (seconds, only when a 429 or 503 sent the header). A `refused` row's `reason`
  is `declined`, `payment_required`, or the verbatim `robots_disallowed` /
  `robots_unavailable: …` text, so the sub-cases stay countable apart.
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
of compression Section 1 of this document says AgentCount exists to
refuse, committed by AgentCount itself. Rung 4 now classifies every field
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

**Implemented 2026-08-01.** This section previously specified a rung no code
executed. Three parts of that specification changed when it shipped, and all
three are listed under "What changed from the specification" below rather
than silently overwritten.

**Question:** Does anything answer at the service endpoints the document
declares?

- **Pass:** at least one probeable declared endpoint answered HTTP 2xx **or
  402**.
- **Fail:** every probeable declared endpoint that we reached gave a definite
  answer that was not live — any other HTTP status, or a URL that could not be
  reached by anyone (`ssrf_blocked`: it does not resolve, or resolves only to
  a private, loopback or link-local address).
- **`refused` (added 2026-08-06):** nothing was live, nothing gave a definite
  non-live answer, and at least one endpoint **declined us** — a 429, a 503, a
  401/407 challenge, or a `robots.txt` that did not give us permission. Rung
  2's rule, applied to the same servers by the same predicate so the two rungs
  cannot disagree about one response. A 429 is a statement about our request
  rate, so counting it as liveness would let our own traffic manufacture the
  finding, and counting it as `fail` would blame the agent for our traffic.
  **402 is the one status that differs between the two rungs and it is
  deliberate** — see the paragraph below.
- **Error:** every probeable endpoint failed on **our** side — a timeout, a
  TLS failure, a connection that never opened. Never the agent's fault, so
  never a `fail`.
- **Precedence, for an agent declaring several endpoints:** pass > fail >
  refused > error. One live endpoint answers the question outright; a definite
  non-live answer is the agent's fact and stands on its own; a decline is the
  verdict only when nothing else was learned; `error` is last because it is the
  only one that says nothing about the world.
- **`unprobeable`:** the document declared no service endpoint that a prober
  can dial. Its entries are CAIP-10 chain addresses, email addresses, empty
  strings, `ipfs://` URIs, or carry no `endpoint` field at all — or it
  declared no services. Across the four-chain census **11.0% of all declared
  "endpoints" are not network endpoints**, so this is a large population, not
  a rounding case. See Section 2's status list for why it is its own word and
  not `unclaimed`.
- **Absent (no row):** the agent declares a probeable endpoint but **none of
  its URLs was probed**, because its host's sampling budget was already spent.
  See "Sampling" below. This is the ordinary Section 4 meaning of an absent
  rung — we did not ask — and it is the only honest answer available.
- **Evidence:** `endpoints_declared`, `endpoints_probeable`,
  `endpoints_probed`, `endpoints_live`, `endpoints_payment_gated`,
  `endpoints_answered_not_live`, `endpoints_refused`, `endpoints_our_error`,
  plus an `endpoints[]`
  array carrying, per declared entry: its index, its `name`, the **raw
  declared string**, its classified kind, whether it was probed, and its own
  URL, final URL, status, elapsed time and outcome.
- **Does not mean:** that the endpoint does anything useful, correctly, or at
  all beyond answering. **Liveness is not functionality**, and three cases
  make that concrete: a `GET` returning 404 may front a perfectly working
  POST-only service; a 200 may be a parking page, a login wall or a load
  balancer's default backend; and nothing here checks that the thing
  answering is the agent, speaks any particular protocol, or would do
  anything if asked. This rung does not read the response body at all.

**HTTP 402 is live here, and is `refused` at rung 2.** Both are correct,
because the two rungs ask different questions. Rung 2 asks whether the
registration document could be *retrieved*; a 402 means it could not. Rung 6
asks whether anything is *alive* at a service endpoint; a 402 is a payment
challenge, and a dead host does not bill you. It is counted separately as
`endpoints_payment_gated` so no reader has to take "live" on trust. Anyone
reconciling a 402 count between the two rungs needs this paragraph.

**And why 429 does not get the same treatment**, when "a dead host does not
rate-limit you" is equally true: a 402 is the origin's considered answer to
anyone who asks, while a 429 is a statement about *us* — about how many
requests we sent and how fast. Reading it as liveness would mean the harder we
probe a host, the more live its agents appear. It is `refused` at both rungs.

**Sampling.** 125,705 declared HTTP(S) endpoints across the census resolve to
3,399 distinct hosts, and four hosts carry 59.2% of them. Probing every
declared entry would send 26,273 requests to one server to learn one fact
about it, which is indistinguishable from an attack and would not be more
true for the volume. So:

1. Exact URLs are **deduplicated** — one request per distinct URL, however
   many agents declared it. On the four-chain data this takes 124,364
   declared entries down to 62,243 distinct URLs.
2. A **per-host budget of 500 distinct URLs** is then applied. 12 hosts
   exceed it; the remaining 3,336 are probed in full. That leaves 14,494
   URLs actually requested.
3. Which 500 is chosen by a **fixed FNV-1a hash of the URL**, not by arrival
   order, alphabetical order or chance. The sample is therefore identical on
   a resume, on a re-run, and on anyone else's machine — a sample nobody can
   reproduce is not evidence. Alphabetical order was rejected because a host
   whose URLs are `…/agent/0001`, `…/agent/0002` would be sampled entirely
   from its lowest ids.
4. An agent whose every probeable URL fell outside the budget gets **no
   rung-6 row**. It is *not* assigned its host's sampled rate. Doing that
   would be inventing a status for an agent nobody checked, which is the one
   thing this project's six statuses exist to prevent. Of 56,794 agents
   declaring a probeable endpoint, **27,956 (49.2%) receive a rung-6 row**;
   any published rate is stated over those, and says so.

**What changed from the specification this section previously carried:**

1. **Pass was "every declared endpoint returns 2xx"; it is now "at least
   one".** Sampling forces this: once some of an agent's endpoints may go
   unprobed, "every endpoint answered" is a claim we cannot make for a
   partially-probed agent, and an all-must-pass rule would have had to
   return no row for every multi-endpoint agent it sampled. The any-match
   rule is also the one rung 5 already uses for `registrations`, and it
   matches what the rung is named for: whether the agent is reachable, not
   whether every URL it lists is perfect. Every endpoint's own outcome is in
   evidence with the counts, so a reader who prefers the all-must-pass
   definition can compute it from the published rows without this rung
   having chosen it for them.
2. **Zero declared endpoints was "a rung-4 concern, not this one"; it is now
   `unprobeable` here.** Rung 4 does check whether `services` is present, and
   still does — but leaving rung 6 to say nothing about an agent it plainly
   cannot probe meant conflating "declared nothing" with "was not sampled",
   which are different facts.
3. **The method was "HEAD, falling back to GET"; it is GET only.** A
   meaningful number of servers answer HEAD with a 405 or a 404 while serving
   the same URL correctly on GET, which would have produced false `fail`s —
   an accusation about a real project, from an optimisation.

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
- **Service** (6): depends on rung 4 — a document that conforms enough to
  declare `services`, not the full chain back to rung 1, and *not* rung 5 (an
  agent can decline to bind its document to the chain and still have live
  endpoints worth checking). This dependency was fixed here, and tested,
  while rung 6 was still unimplemented; rung 6 shipped on 2026-08-01 without
  it needing to change.
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
  **P0 FIX 6 (2026-07-29) narrows this to one meaning**: we did not ask.
  Before this fix, "this run did not attempt it" was a second, silently
  overlapping reason an implemented rung could also be absent — rungs 4 and 5
  were only *constructed* when a document existed to judge, so an agent whose
  document never resolved or never parsed got no rung-4/5 row at all,
  indistinguishable from rung 6, which was then unimplemented. That case is
  `skipped` now, not absent — see `CHANGELOG-METHODOLOGY.md`'s FIX 6 entry
  for the defect and its measured population effect. Since 2026-08-01, when
  rung 6 shipped, the one rung that is still legitimately absent for some
  agents is **rung 6, for an agent whose declared URLs all fell outside their
  host's sampling budget** — the same claim as before (we did not ask), for a
  different reason. The other remaining exception is
  an agent absent from the run *entirely* (every rung, not just one) because
  its chain read or database write failed on our side — reported separately
  by `crates/sweeper` as `unreadable`/`unwritable`, never presented as a
  per-rung gap. **An absent rung is never a claim that the agent failed it.**
  The schema enforces this at the storage layer: a row is written only when a
  check actually ran, so there is no default value to accidentally read as a
  verdict (no `COALESCE(x, false)` anywhere in this pipeline). If you do not
  see a rung for an agent, the honest reading is "not yet checked," full
  stop — not "presumed to fail."
- **`refused`** *(rungs 2 and 6, added 2026-08-06)* — not one of the three
  kinds above either: the rung was asked, the request was made or deliberately
  withheld, and **the origin declined us**. HTTP 429 or 503 ("not now", the two
  statuses `Retry-After` is defined for), a 401/402/407 challenge, or a
  `robots.txt` that disallowed us or that we could not establish permission
  from. It is not `fail`, because nothing here says the document or endpoint is
  unavailable to anyone but us. It is not `error`, because nothing
  malfunctioned — a 429 is an answer we received, and honoring a `robots.txt`
  is a decision we made. It is not `pass`: we did not get what we asked for,
  and everything above it on the ladder is `skipped` exactly as before.

  **This is the one status addition that moved existing agents.** `unclaimed`
  and `unprobeable` named cases nothing had ever been written for; `refused`
  took rows out of `fail` and `error`. Every archived run has been re-judged
  from its own recorded evidence so the series says one thing — see
  `CHANGELOG-METHODOLOGY.md`'s 2026-08-06 entry for the per-run before/after
  counts, and `DATA.md` for what the already-published archives contain.
- **`unclaimed`** *(rung 5 only, added 2026-07-29)* — not one of the three
  kinds above, and deliberately its own word: the rung was asked, it ran to
  completion, and it found nothing to check, because the agent made no
  binding claim (no `registrations` array, or an empty one) for it to verify.
  It is not a consequence of a lower rung failing (`skipped`), not an absence
  of any row (a rung-5 row always exists for an agent that reached rung 5),
  and not a checker malfunction (`error`) — see Rung 5's entry in §2 for the
  full reasoning.
- **`unprobeable`** *(rung 6 only, added 2026-08-01)* — the same shape of
  claim as `unclaimed`, one rung over: the rung was asked, it ran to
  completion, and it found nothing it could reach, because every endpoint the
  agent declared is something no prober can dial (a CAIP-10 chain address, an
  email address, an empty string, an `ipfs://` URI, or no `endpoint` field at
  all) — or the document declared no services. It is deliberately **not**
  `unclaimed`: an agent that published a CAIP-10 address made a claim, it is
  simply not one you can send a request to, and collapsing the two would
  erase that difference exactly as folding `unclaimed` into `fail` would
  have. It is also not `fail`, because the spec does not require an agent to
  publish a URL.

**Current implementation status, stated plainly:** rungs 1 (`registered`),
2 (`resolvable`), 3 (`parseable`), 4 (`conformant`), 5 (`bound`), and 7
(`attested`, renamed from `independent` 2026-07-29) are implemented and run
in every sweep. Rung 7 runs for every agent that passes rung 1 — essentially
the whole population, not the small rung-1-through-5 intersection it used to
be gated on; the only agents it produces no row for are the rare case where
rung 1 itself fails (an owner-is-zero-address token) and the read is never
attempted, plus any agent where the chain read failed on our side (counted
and reported, never silently dropped — see `crates/sweeper`). **Rung 6
(`live`) shipped on 2026-08-01** and writes a row for every agent except two
populations, both of which are absences rather than verdicts: an agent whose
declared URLs all fell outside their host's sampling budget, and an agent
whose rung-4-passing document left no archived body to read `services` from.
It is produced by a separate pass (`crates/sweeper`'s `liveness` binary) over
a run that has already finished, because its unit of work is a URL and the
sweep's is an agent. Rungs 4 (`conformant`) and 5 (`bound`) are
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

A pinned spec goes stale silently, so it is re-checked against the standard
rather than assumed: **as of 2026-07-30 the pinned text (`68fc676`) is
byte-identical to ERC-8004 as published in `ethereum/ERCs`, so no result in
this census was judged against a superseded version of the standard** — the
canonical text's last substantive change was 2026-01-25, five months before
the pin. Every such check, and how to repeat it, is recorded in
[`spec/SOURCE.md`](spec/SOURCE.md).

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

### Clean commits, and who wrote the export

Two provenance rules, adopted 2026-08-02 after this project's own review
found the published index disagreeing with the database about which checker
produced the four canonical runs.

**Canonical runs are swept from clean commits only, from the next sweep
onward.** Three of the four July 2026 runs were built from uncommitted trees
and carry a `-dirty` checker commit. The stamp is honest, but a `-dirty`
build cannot be checked out and rerun by anyone, including us, which is a
poor foundation for a census whose claim is recomputability. The July runs
stay published as they are — the stamp says what happened — and no future
run is published from a dirty build.

**Manifests record the checker and the exporter separately.** A rebuilt
export is written by a different build than the sweep it rebuilds.
`checker_version`/`checker_commit` are the sweep-time values from the `runs`
table and answer "what judged this run"; `exporter_version`/
`exporter_commit` name the binary that wrote the files. The four July
archives predate this rule: their internal manifests carry the export era's
`schema_version: 7` / `checker_version: 0.6.0` in the checker fields, which
is wrong — the database, the API, and `published-runs.json` record the
sweep-time `5` / `0.5.0`, and the archives are immutable so their manifests
keep the error. Where the two disagree, the database is authoritative. The
correction is logged in [`CHANGELOG-METHODOLOGY.md`](CHANGELOG-METHODOLOGY.md).

### The continuous registration tail is not a measurement

Added 2026-08-03. A census pins a block, and that pin is where its authority
comes from: a figure is a statement about a population that existed
*simultaneously*. The cost is that an agent minted after the sweep does not
exist as far as this site is concerned — searching finds nothing, and its
permalink returns 404. The person most likely to hit that is the registrant,
minutes after minting.

So AgentCount also runs a **registration tail**: a poller that reads the
registry's current highest agent id (the same contiguous-id binary search over
`ownerOf` the census uses) and records, for each id above the last sweep's,
only what an on-chain read gives cheaply — the owner, the URI it declares, and
the block both were read at. Those rows make a new agent findable and
linkable. The boundary around them is absolute:

- **A tail row carries no check results.** Not `pass`, not `fail`, not
  `skipped`, not an empty list. No document was fetched and no rung was
  answered, so there is nothing to report and nothing to infer. The API
  returns tail agents in a shape that has no rungs array at all, marked
  `"source": "tail"`, so no client can render seven statuses for an agent that
  has none.
- **No census figure includes tail data, ever.** Every population count, base
  rate, finding, delta and archived export is computed by joining down from a
  `runs` row. The tail's table (migration 0018) has no `run_id` and no foreign
  key to `runs`, so those queries cannot reach it — the separation is
  structural, not a filter someone has to remember to write.
- **A tail row is a receipt, not an observation of the population.** It says
  "the registry contained this id at this block". It does not say the agent was
  present at any census's pinned block, and it is never added to one to make a
  more current-looking total. When a later sweep covers the id, the agent's
  real answers come from that run, and the tail row is marked superseded and
  stops being served.
- **"Discovered at" is when we looked**, not when the agent was minted. The
  poller reads state, not logs, so it knows the block it read at and nothing
  about the registration transaction. Registration provenance (`minter`,
  `registration_tx_hash`, `registration_block`) is captured by the census, from
  logs, and stays there.

The number of unswept tail agents is published per chain (`/api/tail/summary`)
precisely so the gap between the pin and now is visible rather than papered
over. It is a count of things this census has **not** measured.
### Spot checks are not measurements of record

Added 2026-08-03. Any agent page offers a **spot check**: a button that runs
the ladder for that one agent against the chain's current head and shows the
answer immediately (`POST /api/agents/{chain}/{id}/spot-check`). It exists
because someone who has just fixed their registration document should not have
to wait for the next sweep to find out whether the fix took.

**A spot check is not a run, and its answer never enters a published figure.**
That is a structural claim, not a promise to be careful:

- It has no `run_id`, because it belongs to no run. No rate, no finding, no
  `/data` archive and no published count is computed from it, and none can be:
  the endpoint writes no row anywhere, so there is nothing for a query to
  reach. `crates/api/src/routes/spot_check.rs` carries the full argument,
  including why storing nothing was chosen over storing in a separate table.
- Its response carries `"source": "spot_check"` and a notice saying so in the
  body, alongside its own `checked_at`, `block_number`, `checker_version`,
  `checker_commit`, `schema_version` and `spec_commit` — the same provenance
  set a run stamps, so a spot check is as reproducible as anything else here.
  It simply describes a different moment. Its response shares no top-level
  field name with the census's agent detail, so a screenshot of one cannot be
  passed off as the other.
- The pin is the difference that matters. A census figure is *N agents as they
  existed simultaneously at block X*. A spot check is one agent at whatever
  block the chain had reached when somebody clicked a button. Both are true;
  only the first is a statement about a population, and averaging spot checks
  would produce a rate whose denominator is "agents somebody happened to click
  on" — the most self-selected sample imaginable.

**The verdicts are the census's verdicts.** Every rung comes from
`crates/checks`, through the same functions the sweeper calls, and the gating
between them from `checks::run_ladder` and nowhere else. A spot check and a
census row can disagree about the world — a server that was up in July may be
down today, which is the entire point of the feature — but they can never
disagree about what a status means.

**Rungs 1 to 5 and 7 are asked. Rung 6 (`live`) is not**, and its absence from
the response means what absence always means here: not checked, never a
guessed status. The response says so explicitly, with the reason. See Section
6 below for what that reason is.

## 6. Probing etiquette

Rungs 2 and 6 fetch resources we do not control: an agent's declared document
and its declared service endpoints. This is the policy that behavior commits
to. The probe layer described below (`crates/probe`) implements the fetching,
robots.txt handling and redirect-following for **both** — rung 6 shipped on
2026-08-01 and reuses rung 2's probe unchanged, so every guarantee in this
section applies to service-endpoint requests exactly as it does to document
fetches. Where an older, retired component in this codebase used to behave
differently, that's called out rather than glossed over.

- **User-Agent:** every request will identify itself, e.g.
  `agentcount-probe/0.2 (+https://agentcount.ai/methodology; contact: probes@agentcount.ai)`
  — never disguised as a browser. A predecessor component
  (`crates/enricher`) sent an unreachable-by-design User-Agent
  (`ledgerscope-observer/0.1 (+https://ledgerscope.example/methodology)`,
  `.example` being a reserved TLD that can never resolve). That crate has
  since been deleted and the placeholder retired with it; this document
  existed in part to force that fix, and did.
- **Contact:** `probes@agentcount.ai`. If our traffic is a problem for you —
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
- **A per-host cadence, not only a per-host cap (added 2026-08-06).** A
  concurrency cap bounds how many requests a host is answering at once; it does
  not bound how many it answers per second, and a host that replies in 20ms was
  therefore seeing about 100 requests a second from us. That is how the 2026-08
  census collected **19,658 HTTP 429s from one host** — and then published them
  as agents that had stopped resolving. So a host now sees **at most one
  request started every 250ms**, on top of the existing cap of 2 in flight:
  four requests a second, per host, whatever the rest of the sweep is doing.
  Four hosts carry 59.2% of every declared endpoint in the census, so this is
  the limit that decides what those four experience.
- **`Retry-After` is honored** on the two statuses it is defined for (429 and
  503). The value pushes that host's next request forward, so the rest of the
  sweep backs off the host that asked. We do **not** retry the request that
  received it: a sweep is one fetch per agent per run, and retrying would
  double the traffic aimed at the host that had just asked us to stop. A
  backoff longer than **120 seconds** is capped at that, because one host must
  not be able to hold a share of a sweep for an hour — the value the origin
  asked for is recorded in the rung's evidence (`retry_after`) either way, so a
  reader can see we were asked for more than we gave.
- **Rung 6 is stricter than one-per-agent, deliberately.** It is one request
  per *distinct URL*, not per agent, and above that a **per-host budget of
  500 distinct URLs per run**. Without it the largest operator in the census
  would receive 26,273 requests from us in one sweep. With it, no host sees
  more than 500 — and the 3,336 hosts below that number are probed in full.
  The cost is that agents beyond the budget get no rung-6 row; see Rung 6 in
  Section 2 for why that is the honest outcome rather than a gap to fill in.
- **Methods used:** HTTP GET, for both rung 2 (fetching the registration
  document) and rung 6 (checking each declared service endpoint). Rung 6 was
  specified as "HEAD, falling back to GET" before it shipped; that was
  dropped because a meaningful number of servers answer HEAD with a 405 or a
  404 while serving the same URL correctly on GET, and the saving is not
  worth publishing a false `fail` about a working service. Probing does not
  crawl beyond the single declared document and the service endpoints it
  lists — nothing here is meant to spider a site.
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
- **An unavailable `robots.txt` means we do not fetch, and the result is
  `refused` (policy fixed 2026-08-06).** When `/robots.txt` answers 5xx, times
  out, refuses the connection, returns something that is not UTF-8, or
  redirects in a loop, we cannot establish permission. Three policies were
  available and the choice matters, so it is stated rather than left to the
  code:

  1. *Treat unavailable as permission granted* — the common crawler
     convention, and what would make our error rate look best.
  2. *Treat it as denied* — RFC 9309 §2.3.1.4's conservative reading.
  3. *Treat it as its own non-error observation.*

  **We do 2 for behaviour and 3 for the record**, and we did not take 1. A
  robots.txt is the only channel a site operator has for saying no without
  knowing we exist, and reading its failure as a yes takes the benefit of the
  doubt in the direction that suits us — on hosts that are, by definition,
  having trouble serving requests. The conservative behaviour is unchanged from
  the day this project started; what changed is that the outcome is now
  recorded as `refused` rather than `error`. Calling it `error` claimed this
  checker had malfunctioned, which it had not, and made the published error
  rate a measure of one host's `/robots.txt`: **22.1% on the 2026-08 mainnet
  run, 6,133 agents of it a single host refusing connections on that one
  path**. The cost of the conservative policy is stated plainly: those agents'
  documents go unread, and we say so per row rather than absorbing it into a
  number about ourselves. An operator whose robots.txt is unreachable and who
  wants to be crawled can fix it, or mail the contact address above.
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
- **On-demand spot checks are the tightest budget here** (added 2026-08-03;
  see Section 5). A spot check is a request a stranger can cause us to send,
  so it is limited twice: at most **five spot checks per ten minutes from one
  caller**, and — the limit that actually protects you — **at most one per
  minute and twenty per hour to any one host**, counted on the host of the
  URI the agent published on chain. Rotating callers or agent ids does not
  move the second number: whoever asks and however many ids they use, a host
  receives at most one spot-check request per minute from us in total. Over
  the limit returns `429` with a `Retry-After`.
- **A spot check never probes rung 6.** It fetches the single registration
  document (rung 2) and stops. Rung 6 sends one request per *declared*
  endpoint, a document may declare any number of them at any hosts its author
  chose, and there is no run to apply the 500-URL per-host budget against —
  so an on-demand rung 6 would be an unbounded burst aimed at a target the
  caller picks rather than the census. Rung 6 answers come from runs, under
  the budget above, or not at all.

**Confirmed live 2026-07-30.** `agentcount.ai` is registered and
`probes@agentcount.ai` was tested end-to-end and delivers. This section
previously carried a standing note that it must not ship until that was true;
the note is gone because the condition is met, and this paragraph replaces it
so the history of the commitment stays visible rather than silently
disappearing.

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
attached to it — email `probes@agentcount.ai` with the run id and agent id
and we will look at it.

## 8. Payments — the attribution rule

Added 2026-08-06. This section describes a measurement that is **not a rung**
and has produced **no published figure**. It is here before any number exists,
for the same reason the rest of this document was: so the method can be checked
before the result, not after.

### What is being measured, and what it is not

One question: **has an agent's payment address ever received a stablecoin
transfer, up to the run's pinned block?** Not "has this agent earned money".
Direction is not purpose — an incoming stablecoin transfer is indistinguishable
on-chain from an airdrop, a refund, a mistake, or an operator moving its own
funds — and nothing in this pipeline claims otherwise.

It is a separate table (`payments`, migration 0019), a separate binary
(`payments <chain> [run_id]`) and a separate section because it is a question
about token transfers, judged against no clause of ERC-8004. Putting it in
`check_results` would make it the eighth rung by placement whatever the words
around it said.

Everything else about it is the same as a rung: it is scoped to a `run_id`,
every read is at that run's `pinned_block`, excluded rows are stored with the
reason they were excluded, and re-running the binary against the same chain
state reproduces the same rows.

### THE RULE

> **An incoming token transfer may be attributed to agent *A* only if it was
> credited to the address `getAgentWallet(A)` returned by the Identity Registry
> at the run's pinned block, that address is non-zero, and that address is not
> equal to `ownerOf(A)` at the same block.**

Two addresses could have been chosen, and they are not equivalent:

| | `getAgentWallet(agentId)` | a `services[]` entry named `agentWallet` |
|---|---|---|
| where it lives | on-chain, reserved registry metadata | the off-chain JSON document |
| in the pinned spec? | **yes** — `spec/ERC8004SPEC.md` line 141 | **no** — appears nowhere |
| who can set it | the owner, **only by proving control of the new address** with an EIP-712 signature (EOA) or ERC-1271 (contract wallet) | anyone who can write the document |
| on NFT transfer | **cleared automatically**; must be re-verified | survives indefinitely |
| can it name an address nobody controls | no | **yes**, and one does |

The registry's address is used, for four reasons:

1. **Only one of them is a payment address at all.** The spec reserves
   `agentWallet` and defines it as the address where the agent receives
   payments. `services[]` is a list of service descriptors; nothing in the spec
   reserves that name inside it or gives it payment semantics. The convention
   is real and widely used — it is measured, below — but measuring it measures
   adoption of a convention, which is a different finding.
2. **Only one of them carries a proof.** A declared address has no proof of
   control, no link to the on-chain identity, and is served over mutable HTTP.
   On Base, **409 of 919** declared addresses disagree with the address the
   registry has verified, and a further **50** are addresses the registry has
   never verified at all. Every one of those has an innocent explanation. **A
   payer cannot tell which.**
3. **Only one of them can be checked against the population it describes.** A
   `services[]` entry can name any string. Mainnet agent 28283 declares the
   zero address; a scan that trusted it collected **313,255 of mainnet's
   314,735 transfers (99.5%)** — every USDC and USDT burn on Ethereum — as that
   agent's income. Nothing can sign for the zero address, so
   `setAgentWallet` cannot produce that row.
4. **Only one of them self-invalidates.** `agentWallet` is cleared when the NFT
   is transferred, so a stale address cannot outlive a change of owner. A
   document can, and does: an attempt to derive the wallet set from
   `MetadataSet` events produced 19,570 Base agents, of which a sampled **~99%
   were stale**, because clearing emits no event.

### Why "distinct from the owner" is part of the rule

`getAgentWallet` defaults to the owner's address. On Base at block 49,262,617,
**40,473** agents have it set — and **40,126 of them (99.1%) to the owner's own
address**. Only **347** verified a distinct one.

A default is not a claim. Those 40,126 required no `setAgentWallet` call and so
no signature from anybody. They are also not per-agent: one Base owner holds
**2,293** agents, so a transfer to that address is evidence about an operator
and cannot be assigned to any one of its agents without inventing the
assignment.

So an agent whose verified wallet equals its owner is recorded as
`wallet_equals_owner` — **not** as an agent that received nothing. Those are
different facts and the table keeps them apart. That cohort is reportable at
operator level and this pipeline does not report it at all.

### The looser basis is measured too, and never published

Every run also computes the `services[].agentWallet` convention, stored in the
same tables under `basis = 'declared_wallet'`. It is computed because **the gap
between the two is the most honest thing this measurement produces**, because
the prior study is only comparable on it, and because declining to compute it
would be a decision about what readers get to see. It is never the headline,
never blended with the verified basis into one count, and carries its own
column so no query can union the two by accident.

### The four exclusions

Each is a named rule, stored on the row that it excluded, so the uncorrected
figure stays recomputable and each correction is visible rather than asserted.
They are applied in this order and the order is fixed by a test.

| rule | what it removes | why it exists |
|---|---|---|
| `outgoing` | the address sent, rather than received | never a payment to anyone |
| `burn_address` | the credited address or the sender is `0x0…0` or `0x…dead` | a transfer *to* the zero address is a burn, the opposite of revenue; a transfer *from* it is a mint |
| `self_transfer` | the credited address paid itself | an address must not be able to manufacture its own payment history |
| `owner_funding` | the sender is the agent's NFT owner at the pinned block | an owner funding its own agent's wallet is not income |
| `mint_block_unknown` | the agent's registration block is not recorded | whether it predates the mint is unknowable, and the unknown is excluded rather than assumed favourable |
| `pre_mint` | the transfer arrived before the agent was minted | a wallet's history before the agent existed is not the agent's history |

`pre_mint` is the largest of them. On Base it removes **6,748 of 18,328**
transfers by count and **82.5% of all value** — the difference between the
retracted "$8.8M received" and the corrected figure.

### What is recorded per transfer, and why

Beyond the transfer itself, four facts that the retractions showed were
load-bearing:

- **Which address was credited, and on which basis.** Plus how many agents in
  the run reach that address: the map is many-to-many (one Base address is
  declared by **62** agents), so "addresses paid" and "agents whose address was
  paid" are different numbers and both are reported. For a shared address the
  census can say the address was paid; it **cannot** say which agent the
  payment was for, and it says so.
- **Which token, as the contract described itself.** `symbol()` and
  `decimals()` are read from each contract at the pinned block and stored on
  the row, never assumed. BSC's USDC and USDT are **18** decimals, not 6, and
  Celo's `0x765DE816…` — documented for years as cUSD — now answers **`USDm`**
  at 18. Carrying Base's 6 across four chains would have overstated BSC by a
  factor of 10¹².
- **Whether the sender has code.** 94% of Base's corrected value arrived from
  contracts, and for the largest holder it was provably DeFi vault yield —
  an operator's own capital returning, read as revenue. A sender whose code was
  **not read** is stored as `NULL`, never as `false`, and no contract-vs-EOA
  share is computed while any sender is unread.
- **Whether an EIP-3009 authorization co-occurred.** `transferWithAuthorization`
  emits `AuthorizationUsed(address indexed authorizer, bytes32 indexed nonce)`
  from the token contract in the same transaction as the `Transfer`. That is the
  x402 settlement signature and the only protocol-level evidence of payment
  anywhere in this pipeline. The authorizer is stored alongside the flag, and
  whether it equals the transfer's sender, so a reader can check the one
  hypothesis that would over-count it — a batching contract paying many
  recipients under one authorization — from the rows rather than by sampling.

### The scope, and therefore the direction of every error

Two stablecoins per chain, ERC-20 `Transfer` logs only, one chain at a time.
Native gas tokens, every other ERC-20, every other chain and every off-chain
settlement are invisible. So a count here is a **lower bound on agents paid**.

At the same time, `owner_funding` is one hop rather than a funding graph — an
owner routing through a fresh intermediary is not caught, and a *previous*
owner is not caught either — so the same count is an **upper bound on agents
that earned**. Both bounds are stated whenever a figure is.

### No figure is published under this section yet

The three most quotable payment numbers this project has produced —
**358 agents paid**, **313**, and **190** — came from a one-off log study that
is not in the database, not pinned to a block, and not reproducible by a sweep.
All three are **superseded and unpublishable**. They are not restated here,
because this is a new measurement rather than a correction of an old one, and
the first number it produces will be the first number it has ever published.

That number comes from a run a maintainer schedules. Until one exists, the
absence of a payment figure on this site means exactly what an absent rung
means: **not measured**.

## 9. Deltas — what changed between two sweeps

Registration counts go up, and everyone publishes them. **Agents that stopped
resolving** is the number nobody else can produce, because it requires having
asked the same question of the same population at two pinned blocks and kept
both answers. This section defines that number and the series around it,
because a figure with no published method is a claim, not a measurement.

### The pair

A delta compares **one run against the previous finished run on the same
chain** — the pair is chosen when the sweep finishes and recorded permanently
(`run_id`, `previous_run_id`), so "previous" can never silently re-bind as
later runs land. A run with no predecessor gets **no delta at all**: "first
observation" and "nothing changed" are different claims, and a row of zeroes
would read as the second. For the same reason the API serves a missing delta
as an absence (404), never as zeros.

### The series

Over the agents with check results in each run of the pair:

| Series | Definition |
|---|---|
| `newly_registered` | Present in the newer run, absent from the older. |
| `disappeared` | Present in the older run, absent from the newer. Expected to be 0 — an ERC-721 is not usually burned — so a non-zero value is a finding. |
| `newly_resolving` | Rung 2 (`resolvable`) moved from a not-pass to `pass`. |
| `stopped_resolving` | Rung 2 moved from `pass` to a not-pass. |
| `flips` | Every (rung, from-status, to-status) transition with the number of agents that made it. Complete — including everything the series above exclude. |

Three things are deliberately **not** counted as change: an agent present in
only one run of the pair contributes to `newly_registered`/`disappeared` and
to nothing else; a rung with a result on only one side is not a flip ("we did
not ask" is not a change in the world); and — the rule with two incidents
behind it — **a transition into or out of `refused` or `error` is excluded
from `newly_resolving` and `stopped_resolving`**.

A `refused` (429, 503, an auth/payment challenge, a robots.txt that declined
us) means the origin declined the probe, which is not the agent having gone
away, and not the agent having come back. The 2026-08 census briefly reported
19,983 BSC agents stopped resolving; 19,962 were HTTP 429s this census itself
caused, 19,658 from a single host. Excluding transitions touching `refused`,
that chain lost **10** agents (see the 2026-08-06 methodology changelog
entry).

An `error` means **this checker failed to complete the probe** — a timeout,
a TLS or DNS failure, a connection that never completed (§4 defines the
vocabulary: `error` is ours, never the agent's). The 2026-08-17 Base sweep
ran ~17 hours through a degraded network — its own RPC calls timing out all
night — and its delta booked 4,479 Base agents as `stopped_resolving`, of
which 4,477 were `pass → error`: a checker-side outage published as agents
going dark, the 19,983 mistake one status over (see the 2026-08-18 changelog
entry). The exclusion is symmetric here too: `error → pass` is the prober
recovering, not an agent returning.

The cost of the second exclusion is stated plainly: a server that vanishes
outright also surfaces as `error` (one observer cannot distinguish a dead
server from its own unreachability), so these series now **undercount true
disappearances rather than ever overcounting them** — the direction of error
this census chooses everywhere. The excluded transitions remain in `flips` —
deleting the evidence would make a rate limit or an outage invisible, which
is the same failure in the other direction — and the API totals both volumes
(`rung2_declined`, `rung2_errored`; additive, no transition in both) so each
exclusion is visible rather than silent.

### The confound, and the rule for publishing

Each delta records the checker version and evidence schema of **both** runs
(`checker_before/after`, `schema_before/after`). When they differ, an unknown
share of the flips is a method change rather than a change in the world — the
first delta computed on real data showed 564 agents "stopped resolving" across
a checker fix, and publishing that as decay would have been quotable and
wrong. **Any surface that renders a delta must say when the method changed
across the pair.** The API serves all four fields plus a precomputed
`method_changed` so no consumer has to remember the comparison.

### Recomputing one

A delta names both runs; each run names its pinned block, checker commit and
rerun command (§5). Recomputing `sweeper::delta::compute` over the two runs'
check results — the same function the sweep itself calls — reproduces every
series and every flip, byte-identically (flips are sorted). A delta read from
the API and one recomputed from the two published archives cannot disagree
without one of the archives having been altered.

## 10. The Seller Census — Instrument 02

Everything above describes the registration census: whether the agents
everyone counts are real. This section locks the method for the second
instrument: whether the x402 economy everyone cites is real. It follows the
same order as everything else in this project — the method is published
here, in full, before the first seller is enumerated and before the first
cent is spent. The design and its recorded decisions live in
[`analysis/seller-census-design.md`](analysis/seller-census-design.md); this
section is the subset that is LOCKED, meaning a change to any rule below is
a methodology-changelog event, never a quiet edit.

### 10.1 The unit

A **seller** is a deduped **(payTo, host)** pair:

- **payTo** — the payment-receiving address named in a resource's 402
  payment requirements, normalized per network (EVM addresses lowercased).
- **host** — the full lowercase host of the resource URL, port stripped when
  it is the scheme default, IDN in punycode. The full host, not the
  registrable domain: `api.example.com` and `example.com` are different
  services.

The same payTo behind two hosts is two sellers; the same host quoting two
payTos is two sellers. A seller that rotates its payTo is a new seller by
this definition, deliberately: the rotation is information, and host-only
identity would blend genuinely distinct sellers behind shared hosts. Shared
payTos across many sellers are published as findings over the population,
never as merges of the unit. A seller has **resources** — the individual
priced URLs below its host that name its payTo — and both counts are
published; neither stands in for the other.

### 10.2 The population

Sellers are enumerated from named catalogs, because every catalog is partial
and nobody publishes the union. The catalog list is part of this method:
adding or removing one changes the population and is a changelog event, and
a seller whose only catalog was removed is a method change, not churn. Every
sweep archives each catalog's raw response bytes, hash-committed exactly as
run archives are (`DATA.md`), and each seller's row records which catalogs
list it. The self-declaration conventions (`.well-known`, OpenAPI payment
extensions) only ever enrich hosts a catalog already named — crawling the
open web for payment hints has no stopping rule and therefore no defensible
population claim.

### 10.3 The questions

Statuses reuse this document's vocabulary verbatim — `pass`, `fail`,
`error` (ours, never the seller's, §4), `refused` (the origin declined us),
`skipped` (a prerequisite did not pass) — plus one word this instrument
needs: **`unprobed`** — we chose not to ask, and the row says why. `error`,
`refused` and `unprobed` are never publishable as a seller's failure.

An `unprobed` row on rungs 2–3 carries the reason `host_budget`: the seller
sat past this sweep's per-host probe budget (§10.4). The four purchase-side
reasons are listed in §10.4; a row never carries a reason from the other
side's list.

**HTTP 402 is not a refusal in this instrument.** The registration census
reads 402 as `refused` — an agent's document behind a payment wall is a
document we were declined (§4). For a seller, a 402 is the product: the
protocol saying what this costs and where to pay. The statuses that count as
the origin declining us are therefore **401, 407, 429 and 503** — the
registration census's list minus the one status this instrument exists to
receive. Reading them the same way would book every correctly-functioning
seller as having refused us.

**The probe is a GET, and that is a stated limit on rung 3.** One request
answers rungs 2 and 3, because asking twice would double this census's
traffic to every seller for no additional fact. But some resources are
POST-only — an LLM completions endpoint, for instance — and a GET to one may
draw a 402 with an empty body where the declared method would have drawn
full payment requirements. Those sellers are recorded as `fail` with reason
`no_accepts`, which is true of what this census asked and may understate what
the seller does. The count is published beside the rate it qualifies, and
using each catalog's declared method is the stated way to close the gap when
the population justifies the extra care.

This is not a strict ladder; each rung names its prerequisites.

| # | Rung | Question |
|---|------|----------|
| 1 | `listed` | Which catalogs list it, and since when. The population is the listed, so this rung is evidence rather than a verdict. |
| 2 | `reachable` | Does the host answer at all? Any HTTP response — including 4xx/5xx — is `pass`; the question is existence, not health. |
| 3 | `quotes` | Does ≥1 resource return a spec-valid 402 naming a scheme, network, amount, asset and this seller's payTo — judged against a pinned x402 spec commit, exactly as rung 4 of the registration census pins ERC-8004. |
| 4 | `delivers` | Given a real payment (§10.4), does it serve the resource? `fail` here means a payment settled and the resource did not arrive, and carries the settlement proof and the full response. |
| 5 | — | **Reserved.** `receipted` (offers/receipts per the x402 extension) is designed but not in the locked method; it enters by changelog once the extension stabilizes. |
| 6 | `settled` | Does the payTo have on-chain settlement history at a pinned block — facilitator-agnostic, with first/last settlement, count, and distinct payers? Our own shopper payments (§10.4) are excluded by wallet address. |
| 7 | `consistent` | Does what the catalogs claim (price, description, schema) match what the endpoint quotes, field by field? A seller in two disagreeing catalogs is judged against each; the disagreement is itself evidence. |

**robots.txt binds every request this instrument makes** — catalog
enrichment, reachability, the 402 handshake, consistency fetches — with no
carve-out for "the protocol's designed use". A host that disallows us is
`refused`, stated as such. One rule, everywhere, same as §6.

Not measured, on purpose: revenue, dollar volume, uptime, latency, quality
of the delivered resource. No score, no rank, no badge; nothing publishable
is purchasable (GOVERNANCE.md).

### 10.4 The shopper

Rung 4 pays real sellers real money on a schedule, which is only defensible
under pre-registered rules:

- **The wallet is published here, before the first purchase.** Every
  purchase this census ever makes comes from this address, so anyone
  measuring x402 volume can exclude our probes, and our own rung-6 scans
  exclude it mechanically. Rotating it is a changelog event.

  > Base (USDC): **`0x8945b93E68C8927250DDFC41cd10EAc6CbEEd25f`**

- **The cap is $0.10 face value in stablecoin quotes**, inclusive, with the
  face value rounded UP — the only direction that cannot make this census
  pay a price it had decided was too high. A seller not bought from is
  `unprobed`, and the row carries which of exactly four reasons applied:

  | reason | what it says |
  |---|---|
  | `over_cap` | every quote priced above the cap — a price we read and declined |
  | `unpriced` | quoted in an asset this census cannot read at face value |
  | `out_of_scope_network` | quoted only on a network this sweep does not cover (§10.5) |
  | `no_quote` | rung 3 did not pass, so there was nothing to buy from |

  When several apply, the row carries the most informative: a price we read
  and declined says more about a seller than an asset we could not read,
  which says more than a network we do not sweep. Every one of these counts
  is published beside the delivery rate it qualifies — "delivered 61% of the
  83% under cap" is the honest sentence shape, and it needs them.
- **One purchase per seller per sweep**, the cheapest at-or-under-cap
  resource, no retries within a sweep: a payment that settled and a
  resource that did not arrive is the measurement.
- **Politeness is part of the method, not a setting**: per-host concurrency
  of 1, at most 500 probed sellers per host per sweep, `Retry-After`
  honoured with §6's semantics. A 429/503 is `refused` and is never churn.
- **Purchased content is evidenced, never archived**: hash, size,
  content-type, schema-validity against the quote, HTTP metadata. A
  purchased resource is a product; the census stores proof, not copies.
- **Spend is published per sweep**: total, per network, per outcome.

### 10.5 Scope, pinning, change

Sweep 1 covers **Base (USDC)** only. Networks are added the way chains were
added to the registration census: a stated expansion, dated in the
changelog, never silently. Catalog snapshots are hashed per sweep; rung 6 is
pinned to a block; rungs 2–4 and 7 are HTTP facts, timestamped not
block-pinned (§7's honesty applies). Every rate carries its denominator,
absence is served as absence (404, never zeros), and the delta series ship
with the instrument from the first pair of sweeps — with `refused`, `error`
and `unprobed` transitions excluded from headline churn by the same rule §9
records, from day one rather than after the first incident.
