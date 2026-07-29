# Methodology changelog

This document records every change to what Ledgerscope measures or how it
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
