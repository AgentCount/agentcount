# Contributing to AgentCount

Thanks for looking. Bug reports, reproductions, and corrections to our
published numbers are all welcome — the last of those most of all.

## The one rule that is not ordinary

**A change to check semantics requires an issue with a written rationale
before a pull request.**

"Check semantics" means anything that can move an agent's status: the rules in
`crates/checks/`, the field rulings in `spec/REQUIRED_FIELDS.md`, the pinned
spec, or the meaning of `pass` / `fail` / `skipped` / `error` / `unclaimed`.

**Why this rule exists, stated plainly:** everyone whose agent fails a check
has an incentive to argue the check is wrong. Some of those arguments are
correct — several of this project's own checks *were* wrong, and the
corrections are recorded in
[`CHANGELOG-METHODOLOGY.md`](CHANGELOG-METHODOLOGY.md). The rule is not there
to make checks hard to change. It is there to keep the argument in the open,
attached to evidence, so that a check is never softened quietly by whoever had
the most at stake.

A semantic change is complete when it has all four of:

1. **An issue with the rationale**, citing the pinned spec by line where the
   spec is the authority.
2. **A fixture** in `crates/checks` that fails before the change and passes
   after — or the reverse, if you are tightening a check.
3. **A before/after population count**, obtained by re-judging an archived run
   rather than guessing. `http_archive` retains the response bodies precisely
   so this can be done without re-sweeping.
4. **A `CHANGELOG-METHODOLOGY.md` entry** in the existing format: what changed,
   why, and the measured effect.

If a change moves no agent's status, say so and show the count — "measured
effect: none" is a valid and useful result.

## Everything else

Ordinary changes — performance, tests, docs, tooling, API plumbing — need none
of the above. Open a pull request. Frontend and display issues belong in
[`AgentCount/agentcount-web`](https://github.com/AgentCount/agentcount-web);
anything that can move a status is decided here.

## Ground rules for the codebase

- **`crates/checks` stays pure.** No network, no database, no clock. Plain data
  in, a status and evidence out. This is what lets a verdict be re-derived from
  an archived run, and CI enforces it as a named job.
- **Runs are immutable.** A newer answer is a new run. Nothing edits a row in a
  finished run, ever.
- **Six states, never collapsed.** `pass`, `fail`, `skipped`, `error`,
  `unclaimed`, and *absent* (for an unimplemented rung) mean six different
  things. "We could not ask" is not "the agent failed".
- **No aggregate.** No score, grade, tier or ranking, including a
  "N of 7 rungs passed" tally. If a pull request adds one, it will be declined
  regardless of how it is computed.
- **Evidence, not assertion.** A new claim needs something a reader can
  re-check attached to it.
- **New dependencies need a reason in the pull request description.** Not a
  high bar, just a stated one.

## Before you push

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs exactly these, plus the checks-purity gate.

## Reporting a wrong number

If a published figure is wrong, an issue with the run id, the agent id and what
you observed is enough — you do not need a fix. This project has retracted its
own findings more than once before publication, and would rather do it again
than defend a number.

## Contributor Licence Agreement

Pull requests require signing a CLA, handled automatically by CLA Assistant on
your first pull request. It is there so the project can relicense or dual-licence
later without having to track down every contributor. It does not take your
copyright — you keep it and grant a licence.

## Conduct

[Contributor Covenant](CODE_OF_CONDUCT.md). Reports to `probes@agentcount.ai`.
