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
- **Every per-agent query names `chain` — on the tables whose key still leads
  with it.** On `agent_snapshots` and `http_archive`, `chain` is denormalized
  convenience — `run_id` determines it, verified on production as zero runs
  spanning two chains and zero rows disagreeing with `runs.chain`. It is
  **never needed for uniqueness** in a per-run key (though it *is* a leading
  seek column in two secondary indexes, `idx_snapshots_owner` and
  `idx_agent_snapshots_minter`, which exist to make owner and minter lookups
  seek). But it sits between `run_id` and `agent_id` in the per-run key, so a
  `WHERE` that omits it scans the entire run instead of seeking: 8,915 ms
  against 8.8 ms on a 250,000-agent run. Six queries were written that way,
  precisely because the column is redundant and leaving it out reads as tidy.
  The worst of them passed `chain` to an `INSERT` and not to the `DELETE`
  beside it — it was correct, invisible to every test, and 900× slower per
  agent. Only measurement caught it, which is why this is a CI job
  (`scripts/check-chain-predicates.py`) and not a note.

  **`check_results` is no longer one of those tables.** Migration 0025 dropped
  `chain` from its unique key, which is now `(run_id, agent_id, rung)`, so a
  query naming `run_id` and `agent_id` seeks correctly without it and the gate
  no longer asks for it there. That was the point of the migration: it removed
  the reason anyone had to remember. If a future index on `check_results` ever
  leads with `chain` again, put the table back in the gate's list.
- **New dependencies need a reason in the pull request description.** Not a
  high bar, just a stated one.

## Before you push

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

```sh
python3 scripts/check-chain-predicates.py
```

CI runs these as the jobs `fmt`, `clippy` and `test` — note it runs
`cargo fmt --all --check`, which fails on formatting the local command would
have fixed silently — plus three gates you cannot run as one line here:
`checks-purity` (the pure crates gain no I/O, clock or filesystem
dependency), `chain-predicates` (the script above), and `archives-resolve`
(every download link in `published-runs.json` still downloads, which is a
network job and can therefore fail for reasons that have nothing to do with
your change).

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
