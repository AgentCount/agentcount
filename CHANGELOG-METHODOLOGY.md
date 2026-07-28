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
