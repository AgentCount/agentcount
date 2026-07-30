# Governance

**AgentCount is maintained by one person, who has the final say.** This
document says so plainly rather than implying a committee that does not exist.

## How decisions are made

- The maintainer decides what merges, what ships, and what a check means.
- Disagreements are resolved in the open, in issues, on the evidence.
- There is no vote, no steering committee and no formal escalation path.

If the project grows enough that this stops being honest, this document changes
first and the change is announced — not discovered.

## What is deliberately constrained

The maintainer's authority is bounded by rules that exist to stop this project
from quietly becoming the thing it measures against:

1. **Check semantics change in public or not at all.** The process in
   [`CONTRIBUTING.md`](CONTRIBUTING.md) — issue, rationale, fixture,
   before/after count, changelog entry — binds the maintainer exactly as it
   binds a contributor. A check softened without that trail is a bug, and
   pointing at one is a legitimate issue.
2. **Published numbers are not retracted silently.** Corrections carry what was
   claimed, why it was wrong, what it became, and what would have caught it.
   See [`CHANGELOG-METHODOLOGY.md`](CHANGELOG-METHODOLOGY.md) and
   `analysis/payments-corrections-ledger.md` for how that has actually gone.
3. **Runs are immutable.** Nobody edits a finished run, including the
   maintainer.
4. **No aggregate score.** Not by decision, not by pull request, not by
   popular request.

## Conflicts of interest

If the maintainer ever has a commercial relationship with a party whose agents
this census measures, that relationship is disclosed in the report it could
have touched — before the report, not after.

## Commercial use

The code is [Apache-2.0](LICENSE). Anyone may run it, including commercially.
The **name** is not open — see [`TRADEMARK.md`](TRADEMARK.md). A contributor
licence agreement is required for pull requests, which keeps a future
relicensing or open-core split possible without tracking down every
contributor; that possibility is stated here so nobody is surprised by it
later.

## Succession

If the maintainer stops maintaining, the honest outcome is an archived repo
with a note saying so, not a slow decay of unreviewed merges. The published
runs and their evidence remain reproducible from the archive regardless — that
is the point of stamping every row with the commit and spec that produced it.
