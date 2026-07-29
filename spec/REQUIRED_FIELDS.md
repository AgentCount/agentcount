# Rung 4 (`conformant`) — MUST / SHOULD / MAY field classification

Extracted verbatim from `ERC8004SPEC.md` at the commit in `SOURCE.md`
(`68fc6765761a10fb26f0692df21c8a6f9d12b1be`).
Every entry quotes the line that establishes it. If a field is not quoted
here, rung 4 does not check it at all — not MUST, not SHOULD, not MAY.

**Since P0 FIX 3 (2026-07-29), this document classifies every field into
one of three RFC 2119 severities** — MUST, SHOULD, MAY — rather than a flat
REQUIRED/not-REQUIRED list. Only a MUST violation fails rung 4; SHOULD and
MAY gaps are recorded in evidence (`should_gaps[]`, `may_gaps[]`) but never
flip `pass` to `fail`. See `crates/checks/src/rung4_conformant.rs`'s module
doc for the code-level rationale, and the FIX-3 work order for why this
split exists at all: the spec invokes RFC 2119/8174 explicitly (line 36),
and a single boolean was quietly discarding the severity distinction the
spec itself draws.

## Scope: which document this covers

The spec defines **two** distinct off-chain JSON documents:

1. **The agent registration file** — what `agentURI` (aka ERC-721 `tokenURI`)
   resolves to (spec lines 50-124). This is the document rung 4 checks.
2. **The reputation off-chain feedback file** — an unrelated, separately
   OPTIONAL document referenced by `feedbackURI` in the Reputation Registry
   (spec lines 245-288). Its JSON comments literally say `// MUST FIELDS`
   (line 251) and `// ALL OPTIONAL FIELDS` (line 259), but this is feedback
   metadata, not agent identity metadata, and line 245 itself says "The
   **OPTIONAL** file at the URI could look like:" — the whole file is
   optional. It is **out of scope for rung 4** and is not extracted below.

## Extraction methodology (please review)

The spec's RFC 2119 terms are defined at line 36: `"MUST", "MUST NOT",
"REQUIRED", "SHALL", ... are to be interpreted as described in RFC 2119"` —
i.e. **MUST and REQUIRED are the same normative strength in this document**.
The literal string "REQUIRED" appears only once in the whole file (that
definitions line); the document does its normative work with "MUST",
"SHOULD", "MAY", "OPTIONAL", and, in one place, the plain-English synonym
"mandatory". The classification below is built from those keyword
statements, field by field, not from any single governing sentence treated
as a blanket rule.

The registration file's shape is introduced by a single governing sentence
(line 54): **"The registration file MUST have the following structure:"**,
followed by a JSON code block (lines 56-113). **P0 FIX 3's central finding
is that this sentence, read against everything the spec says afterward
about the fields inside that structure, does not make most of them MUST.**
Line 54 establishes the *shape* of the document; later, field-specific
sentences (115, 123, 124) govern the *severity* of individual fields within
that shape, and — per ordinary reading of a spec that states a general rule
and then qualifies it — the more specific, later statement controls. Only
one place in the document does a later sentence leave a MUST-strength
statement intact and unqualified: line 123's "all fields in the
registration are mandatory," which is *itself* the qualifying, specific
statement, and applies only inside `registrations[]` entries, and only when
that array is present. Every other field named at line 54 is downgraded by
a later sentence. That is not a design choice made here; it is what
lines 115, 123, and 124 say.

---

## Rulings

This section is a decision log, not a place earlier entries get edited out
when a later fix reverses them. A reversed ruling stays — the record is
supposed to show that an ambiguity was decided, reconsidered, and decided
again, not that it was quietly fixed.

### Ruling 1 — `type`, `name`, `description`, `image` remain REQUIRED (decided 2026-07-27)

**⚠ REVERSED by Ruling 4 below (2026-07-29, P0 FIX 3). Kept verbatim for the
record; do not treat this ruling as current.**

**Ambiguity:** Line 54 says "The registration file MUST have the following
structure" and that structure includes these four fields. Line 115 then
says these fields "SHOULD ensure compatibility with ERC-721 apps". Two
readings were possible:
- The SHOULD constrains what values those fields should contain (format/content compatibility), leaving presence governed by line 54's MUST.
- The SHOULD downgrades the presence requirement itself to optional.

**Decision:** The project owner has ruled that line 115's SHOULD attaches to "ensure compatibility" (a behavioral constraint), not to "are present" (a presence requirement). Line 54's MUST governs whether the fields appear in the document. All four stay REQUIRED for rung 4.

### Ruling 2 — `registrations` is NOT required, but its sub-fields are validated when present (decided 2026-07-27)

**Ambiguity:** Line 54 says "The registration file MUST have the following structure" and includes `registrations` in that structure. Line 123 then says "Agents SHOULD have at least one registration (multiple are possible), and all fields in the registration are mandatory." Two readings were possible:
- `registrations` is required to be present (per line 54), and any entry inside is mandatory (per line 123).
- Line 123's SHOULD downgrades the presence of `registrations` itself (like it does for non-empty entries), while the phrase "all fields in the registration are mandatory" still governs the contents of any entry that does exist.

**Decision:** The project owner has ruled that line 123's SHOULD applies to the `registrations` key itself (presence is recommended, not required), so a document lacking `registrations` entirely does not fail rung 4. However, the same sentence also says "all fields in the registration are mandatory"—meaning `agentId` and `agentRegistry` are required only within entries of a present `registrations` array. Rung 4 checks these two sub-fields conditionally: if `registrations` is present, every entry must have both; if absent, rung 4 does not check.

**Status: this ruling stands unchanged after P0 FIX 3.** It already reached
the conditional-MUST reading FIX 3 generalizes to the rest of the document;
FIX 3 did not need to revisit it.

### Ruling 3 — `x402Support` and `active` are NOT required (decided 2026-07-28, P0 FIX 2 — reverses the original extraction)

**Ambiguity:** Line 54 says "The registration file MUST have the following structure" and the JSON block that follows includes `"x402Support": false,` (line 99) and `"active": true,` (line 100) as top-level keys, alongside `type`/`name`/`description`/`image`/`services`. The original extraction (this file, first pass) treated every key of that block as REQUIRED unless a more specific sentence downgraded it, and found no such downgrade for these two — so both were listed as unconditionally REQUIRED.

**Why that reading was revisited:** neither field is mentioned anywhere in the spec's prose with a normative keyword (`MUST`, `SHOULD`, `MAY`, `mandatory`, `OPTIONAL`) — they appear *only* inside the illustrative example JSON, at lines 99 and 100, and nowhere else in the document. Treating an example's every key as mandatory is already a strained extension of line 54's MUST (it makes the *structure* of the example normative, not just its documented fields); it becomes especially strained for `x402Support`, where the consequence is that an agent is judged non-conformant for failing to affirmatively declare that it does *not* support a payment protocol — a MUST that would require every agent on the ecosystem to take a position on a specific, optional payment extension. 8004scan's metadata profile classifies both fields as MAY, corroborating that the ecosystem itself does not treat them as mandatory.

**Decision:** the project owner has ruled that a bare appearance inside the example block, with no corroborating normative sentence, does not establish a MUST. `x402Support` and `active` are removed from rung 4's unconditionally-required set and move to "Explicitly NOT checked" below. FIX 3 gives them a formal MAY classification (alongside `supportedTrust`); this ruling only removes them from the required set.

**Status: formalized, not reversed, by P0 FIX 3.** See §MAY below —
`x402Support`/`active` now have an explicit bucket rather than sitting in
"not checked."

### Ruling 4 — `type`, `name`, `description`, `image` move to SHOULD, reversing Ruling 1 (decided 2026-07-29, P0 FIX 3)

**This ruling reverses Ruling 1. It does not delete it — see above.**

**What changed the reading.** P0 FIX 3 required classifying every rung-4
field by RFC 2119 severity rather than a flat REQUIRED/not-REQUIRED split,
which forced Ruling 1's ambiguity to be looked at again, this time next to
every other field-severity decision in the document rather than in
isolation. Two things follow from that wider view that Ruling 1 did not
weigh:

1. **Consistency with Ruling 2's own reasoning.** Ruling 2, decided the
   same day as Ruling 1, read line 123's SHOULD ("Agents SHOULD have at
   least one registration... and all fields in the registration are
   mandatory") as downgrading the *presence* of `registrations` to
   optional, while treating "all fields... are mandatory" as the specific,
   controlling statement for what's inside an entry that does exist. That
   is precisely the pattern of "a later, field-specific SHOULD/MAY
   statement controls over the earlier general MUST" — applied to
   `registrations`. Line 115 draws the same shape for `type`/`name`/
   `description`/`image`: a later, field-specific sentence, using the
   literal word SHOULD, about exactly these four fields. Reading line 115's
   SHOULD as governing only *content* ("ensure compatibility") while
   reading line 123's structurally identical SHOULD as governing *presence*
   is not a principled distinction — it applies the same sentence-shape two
   different ways depending on which fields it's about. Ruling 4 applies
   the *same* rule Ruling 2 already used.
2. **What "SHOULD ensure compatibility" is actually a sentence about.**
   Re-read plainly: "The *type*, *name*, *description*, and *image* fields
   at the top SHOULD ensure compatibility with ERC-721 apps." A missing
   field cannot "ensure" anything — the sentence presupposes the fields'
   existence and evaluates what having them accomplishes. But that
   presupposition is not itself a separate MUST; nothing in the sentence,
   or anywhere else in the document, restates line 54's structural MUST for
   these four fields specifically. The strained move in Ruling 1 was
   treating "SHOULD ensure compatibility" as silently carrying a distinct,
   unstated "and MUST be present" alongside it — reading two normative
   claims out of a sentence that states one.

**Decision:** the project owner has reversed Ruling 1. `type`, `name`,
`description`, `image` move from the (now-empty) unconditional-MUST bucket
to SHOULD. A document missing one, several, or all four no longer fails
rung 4 on their account; each missing one is recorded as a `should_gaps`
entry instead. Outcome is similar in practice to Ruling 1's — either way
these fields end up as "not literally forbidden to omit, but flagged" for
the small minority of documents that omit them, since Ruling 1's MUST
already meant nearly every real document carried them — but the mechanism
differs: under Ruling 1, omitting one was a `fail`; under Ruling 4, it is a
`should_gaps` entry on an otherwise-`pass`ing document. See
`crates/checks/src/rung4_conformant.rs` for the implementation and its own
module-doc citation trail.

**Consequence, stated plainly, not softened:** combined with Ruling 2 (the
only other structural field, `registrations`, already conditional) and
Ruling 3 (`x402Support`/`active` never MUST to begin with), **the
registration file has exactly one MUST left, and it is conditional**:
`registrations[].agentId` and `registrations[].agentRegistry`, checked only
when `registrations` is present at all. Every document that parses as JSON
and either omits `registrations` or supplies complete entries in it now
passes rung 4. This is expected to move the rung-4 pass rate dramatically
upward — see `CHANGELOG-METHODOLOGY.md`'s FIX-3 entry for the measured
figure against the archived census. That is the finding this fix produces,
not an error to be corrected by finding a way to keep the number lower.

---

## MUST — the only bucket that can fail rung 4 (2 fields, conditional)

There is no unconditional MUST field in the registration file after
Ruling 4. When a `registrations` array exists in the document, every entry
in it MUST contain these two fields (line 123: "all fields in the
registration are mandatory"). Rung 5 (`bound`) consumes exactly these two
to validate chain-scoped registration claims.

### `registrations[].agentId`
- JSON path: `$.registrations[*].agentId`
- Spec line: 123, field appears at line 103
- Verbatim: "...and all fields in the registration are mandatory." (line 123), schema line 103: `"agentId": 22,`
- Type: integer (ERC-721 tokenId)
- **Condition:** checked only if `$.registrations` array is present and non-empty; not checked if the key is absent, `null`, or an empty array.

### `registrations[].agentRegistry`
- JSON path: `$.registrations[*].agentRegistry`
- Spec line: 123, field appears at line 104
- Verbatim: "...and all fields in the registration are mandatory." (line 123), schema line 104: `"agentRegistry": "{namespace}:{chainId}:{identityRegistry}" // e.g. eip155:1:0x742...`
- Type: string, colon-separated `{namespace}:{chainId}:{identityRegistry}` (format defined at lines 42-45)
- **Condition:** checked only if `$.registrations` array is present and non-empty; not checked if the key is absent, `null`, or an empty array.

**Missing either field on any present entry is a `must_violations[]` entry
named by its array index (e.g. `registrations[0].agentRegistry`) and fails
the rung. Nothing else in the document can fail rung 4.**

---

## SHOULD (7 checks — 4 simple presence + 3 special-cased)

Absence is recorded in `should_gaps[]`. Never fails the rung.

### `type`, `name`, `description`, `image`
- JSON path: `$.type`, `$.name`, `$.description`, `$.image`
- Spec line: 115 (governing SHOULD, reversing Ruling 1 — see Ruling 4 above); fields appear at schema lines 58-61
- Verbatim: "The *type*, *name*, *description*, and *image* fields at the
  top SHOULD ensure compatibility with ERC-721 apps." (line 115)
- Type: all strings (`type`/`description`/`image` free text or URI, `name`
  a display name)
- **Condition:** checked on every document; each missing field (or `null`)
  is its own `should_gaps` entry, named exactly.

### `services` (legacy alias: `endpoints` — P0 FIX 1, unchanged by FIX 3; reclassified SHOULD by FIX 3)
- JSON path: `$.services`, or `$.endpoints` when `services` is absent
- Spec line: schema line 62, but **no MUST governs it after Ruling 4** — see
  below
- Type: array. Rung 4 checks only presence and non-emptiness — its contents
  remain unconstrained: "The number and type of *endpoints* are fully
  customizable, allowing developers to add as many as they wish." (line 115)

**Why SHOULD, not MUST, and not simply "not checked" either.** The spec
never uses a normative keyword to require `services`/`endpoints` at all —
line 54's MUST establishes the document's overall *structure*, but (per the
reasoning in Ruling 4) that structural MUST does not survive for fields a
later, field-specific sentence addresses, and no such sentence exists for
`services` the way lines 115/123/124 exist for the other four structural
fields. 8004scan's metadata profile is explicit about the practical
reading: `services` is required *conditionally* — "MUST include at least
one service if the agent is meant to be interacted with" — which is exactly
a SHOULD in spirit (satisfiable, but its absence is a real, checkable
deficiency for an agent that wants to be reachable) rather than an
unconditional MUST every agent must satisfy regardless of purpose.

**Legacy alias — why `endpoints` is accepted here (P0 FIX 1, carried
forward).** The schema block (line 62) names the field `services`, but the
surrounding prose never caught up: it says "endpoints" instead, repeatedly —
lines 115, 117, 121, 402 (Rationale), and 416 (inside the commented-out Test
Cases block — noted because the P0 work order cited it, but it is editorial
scratch text, not published normative prose; it doesn't change the
conclusion since 115/117/121/402 already establish the pattern outside any
comment). 8004scan's profile documents this as a January-2026 rename:
`services` and `endpoints` both accepted, `services` wins when both are
present, legacy-only use is warning `WA031`, no deprecation date. Rung 4's
rule: `services.or(endpoints)`; a document is never penalized, at any
severity, solely for using the legacy name.

**Empty vs. absent, recorded distinctly (P0 FIX 3).** The work order asked
for this explicitly: an empty `services`/`endpoints` array is not the same
fact as a missing one. A key that's present with zero entries describes an
agent that supplied the field but is, by its own declaration, unreachable
by any means it advertises — worth tracking on its own, separate from a
document that never engaged with this part of the schema at all. Evidence
records `services_status` (`"absent"` / `"empty"` / `"present"`) always, and
`should_gaps` carries a distinct label per case:
- Absent (neither `services` nor `endpoints` present) → `"services"`.
- Present but zero entries → `"services_empty"`.
- Present, non-empty → no gap for this field.

Evidence also always carries `services_field_source` (`"services"` /
`"endpoints"` / `"neither"`), `legacy_endpoints_field`, and
`both_fields_present`, unchanged from FIX 1, so the population's migration
rate remains queryable without a re-sweep.

### `registrations` — at least one entry
- JSON path: `$.registrations`
- Spec line: 123
- Verbatim: "Agents SHOULD have at least one registration (multiple are
  possible)..." (line 123)
- **Condition:** checked on every document. Zero usable entries — whether
  the key is absent, `null`, present-but-not-an-array, or present as an
  empty array — is one `should_gaps` entry: `"registrations"`. The work
  order asked for the empty-vs-absent split specifically on `services`; it
  did not ask for the same split here, and the single "at least one" phrase
  in the spec doesn't suggest the two failure modes are being distinguished
  for `registrations` the way they are for `services`, so this stays one
  label covering both.

### `services[].version` (or `endpoints[].version`)
- JSON path: `$.services[*].version` (or `$.endpoints[*].version` under the
  legacy alias)
- Spec line: 115
- Verbatim: "The *version* field in endpoints is a SHOULD, not a MUST."
  (line 115)
- **Condition:** checked against whichever entries actually exist (via the
  same `services.or(endpoints)` resolution as the parent field). If *any*
  present entry lacks `version`, this is **one** `should_gaps` entry —
  `"services[].version"` — regardless of how many entries are missing it.
  This is a document-level completeness signal, not a per-entry violation
  list; an agent that declares ten endpoints and forgets `version` on all
  ten has one gap, not ten, the same way a MUST violation would be recorded
  per-entry only because rung 5 consumes that index-level detail — nothing
  downstream consumes per-entry SHOULD detail the same way.

---

## MAY (3 fields checked; 1 work-order field deliberately excluded — see below)

Purely informational. Absence is recorded in `may_gaps[]` and never affects
`pass`/`fail`, and its presence is never itself checked for anything beyond
existence (no validation of `x402Support`'s boolean-ness, etc. — this rung
only ever asks "is the key here").

### `x402Support`
- JSON path: `$.x402Support`
- Spec line: appears only at schema line 99, no normative-keyword sentence anywhere in the prose
- See Ruling 3 for the full reasoning this classification rests on.

### `active`
- JSON path: `$.active`
- Spec line: appears only at schema line 100, no normative-keyword sentence anywhere in the prose
- See Ruling 3.

### `supportedTrust`
- JSON path: `$.supportedTrust`
- Spec line: 124
- Verbatim: "The *supportedTrust* field is OPTIONAL. If absent or empty,
  this ERC is used only for discovery, not for trust." (line 124)
- The only one of the three MAY fields the spec marks OPTIONAL explicitly,
  by name, with a normative keyword.

### `updatedAt` — named by the P0 FIX 3 work order's MAY table, but NOT checked here

**This is a discrepancy between the work order and the pinned spec,
reported per ground rule 1 ("if they disagree with this document, the spec
wins and the disagreement is reported back, not silently resolved") rather
than silently resolved either way.**

The P0 FIX 3 work order's MAY row lists `x402Support`, `active`,
`supportedTrust`, `updatedAt`. The first three are traceable to the pinned
spec (above). `updatedAt` is not: `grep -in "updatedAt" spec/ERC8004SPEC.md`
returns nothing at all — the field does not appear in the line-54 schema
block, nowhere in prose, with or without a normative keyword, at the pinned
commit `68fc676`. This was cross-checked against both sources ground rule 1
names:
- `eips.ethereum.org/EIPS/eip-8004` — confirmed no `updatedAt` field
  anywhere in the registration schema or its surrounding prose.
- `best-practices.8004scan.io/docs/01-agent-metadata-standard.html` — *does*
  define `updatedAt`, as an optional Unix-timestamp field for "freshness
  tracking." This is almost certainly the work order's actual source for
  this entry — a real field in 8004scan's extended metadata profile, but
  not one the pinned ERC8004SPEC.md defines.

Rung 4's own founding discipline (its module doc, since Task 1) is "nothing
added, nothing inferred from outside knowledge of ERC-8004" — every field
this rung looks at, at any severity, must be traceable to the pinned spec
text itself, not to a third-party profile's superset of it. Checking for
`updatedAt` — even as a MAY, even though a MAY gap can never fail anything —
would still mean inventing a check the pinned spec does not support,
exactly the kind of scope creep ground rule 4 of the P0 work order warns
against ("Do not fix anything not on this list... scope creep here is
dangerous"), just applied one level down at the level of an individual
field rather than a whole fix. **`updatedAt` is therefore not checked, in
either direction: its presence or absence never appears in any evidence
array.** If 8004scan's broader profile is ever adopted as an additional,
explicitly-labelled source alongside the pinned spec, this is where that
field would be added — with its own citation, the same as every other
field in this document.

---

## Explicitly NOT checked at all — no bucket, by design

Fields the spec marks with inline `// OPTIONAL` comments on sub-structure
this rung does not walk into, or that sit entirely outside the registration
document's scope:

- `services[].skills` (OASF service entries) — inline comment, line 81:
  `"skills": [], // OPTIONAL`. Not walked by rung 4 at any severity; the
  spec marks it optional at the level of one specific service entry type
  (OASF), not the document as a whole.
- `services[].domains` (OASF service entries) — inline comment, line 82:
  `"domains": [] // OPTIONAL`. Same reasoning as `skills`.
- `updatedAt` — see the MAY section above; not in the pinned spec at all,
  documented there rather than silently omitted.

Out of scope entirely (different document, not the agent registration
file — see "Scope" above), not counted as "NOT checked" for this document:
- The Reputation Registry's off-chain feedback file fields (spec lines
  245-288), including the fields its own inline comments mark
  `// MUST FIELDS` (`agentRegistry`, `agentId`, `clientAddress`,
  `createdAt`, `value`, `valueDecimals` at feedback-file lines 252-257).
  The feedback file itself is introduced as "The **OPTIONAL** file at the
  URI could look like:" (line 245) — the whole document is optional, and
  it is not the registration document rung 4 evaluates.

---

## Step 4 sanity check

`grep -ci "required" spec/ERC8004SPEC.md` → **4** matches:

1. Line 36 — the RFC 2119 definitions sentence itself (defines the term,
   establishes MUST ≡ REQUIRED; not a field-level requirement).
2. Line 206 — "...For IPFS URIs, the hash is not required." (Reputation
   Registry `feedbackHash`; out of scope, different document/registry.)
3. Line 207 — "...so the off-chain file is not required and can be
   omitted." (the feedback file's optionality; out of scope.)
4. Line 312 — "...This field is not required for IPFS URIs."
   (`responseHash` in `appendResponse`; out of scope, Validation/Reputation
   registry function parameter, not an agent-document field.)

**Reconciliation:** none of the 4 literal "required" mentions apply to the
agent registration file rung 4 checks — three are about the Reputation
Registry's optional off-chain feedback file and its optional hash fields,
and one is the definitions sentence. This is expected, not a gap: the
document does not use the literal word "required" to mark agent-document
fields at all. It uses "MUST"/"mandatory" for the one conditional MUST
(line 123), and "SHOULD"/"MAY"/"OPTIONAL" for everything else this document
tracks. After P0 FIX 3, the accounting is:

- **MUST: 2 fields, both conditional** — `registrations[].agentId`,
  `registrations[].agentRegistry` (line 123's "mandatory"), checked only
  when `registrations` is present and non-empty.
- **SHOULD: 7 checks** — `type`, `name`, `description`, `image` (line 115,
  Ruling 4), `services`/`endpoints` (empty-vs-absent, two distinct labels),
  `registrations` at-least-one (line 123), `services[].version` (line 115).
- **MAY: 3 fields** — `x402Support`, `active` (Ruling 3), `supportedTrust`
  (line 124, explicitly OPTIONAL). `updatedAt`, named by the work order,
  deliberately excluded — see above.
- **Not checked at all: 2 fields** — `services[].skills`,
  `services[].domains` (both inline `// OPTIONAL` on OASF sub-structure).

12 fields tracked in total (2 + 7 + 3), 2 more explicitly declared
out-of-bucket. Nothing was missed: the low "required" grep count against a
document that clearly does define field-level normative strength is
explained entirely by this document's word choice (MUST/SHOULD/MAY/mandatory
over the literal word "required"), not by an incomplete extraction.
