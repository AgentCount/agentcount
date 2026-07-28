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
