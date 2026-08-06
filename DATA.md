# The data

Every canonical run of this census is published as a downloadable archive.
Free, no account, no key, no rate limit, no email gate. One URL per run,
immutable once written.

```
https://storage.googleapis.com/agentcount-data/runs/<run_id>.tar.zst
https://storage.googleapis.com/agentcount-data/runs/<run_id>.tar.zst.sha256
```

The full list, with each run's chain, pinned block, provenance and size, is at
[agentcount.ai/data](https://agentcount.ai/data) and in
[`published-runs.json`](published-runs.json) in this repository.

That is the storage bucket's own address rather than a hostname of ours, and
deliberately so. A vanity domain in front of these files would be one more
thing that can expire, misroute or lose a certificate between you and an
archive whose whole purpose is to still be there during an argument. The
bucket URL has no such layer.

## Why this is free

This project's central claim is that every number it publishes can be
recomputed by someone else. That claim is only true if the inputs are actually
downloadable — a census whose data you have to ask for is a census you have to
take on trust, which is the thing it exists not to be.

Data exclusivity was never the point and is not the business. If a paid layer
ever exists it will sell convenience and continuity — a hosted API,
cross-run history, alerts — the way Etherscan and Dune sell those things over
chain data anyone can read for free. It will sit above this, never in front of
it.

## Licence

**Data: [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).** Use it for
anything, including commercially. The one condition is attribution: cite
AgentCount and, ideally, the `run_id` you used, because a figure without a run
id cannot be re-derived by whoever reads your work next.

CC0 was considered and not chosen. Not to restrict anyone — CC BY restricts
almost nothing — but because a census's value depends on findings staying
traceable to the sweep that produced them, and attribution is the mechanism
that keeps that chain intact. If attribution is genuinely a blocker for your
use, open an issue and say why.

The code in this repository is Apache-2.0. Different licences, on purpose:
they are different kinds of thing.

## What is in an export

An archive expands to:

```
<run_id>/
  manifest.json          the run's provenance and its own honesty about gaps
  <chain>/<agent_id>.json    one file per agent swept
```

### `manifest.json`

`run_id`, `chain`, `chain_id`, `registry`, `pinned_block`, `started_at`,
`finished_at`, `schema_version`, `checker_version`, `checker_commit`,
`spec_commit`, `rerun_command`, and four counts:

| field | meaning |
|---|---|
| `agent_count` | the population this run set out to sweep |
| `swept` | how many were read and persisted |
| `unreadable` | read failed on OUR side — absent from the run, not failed |
| `unwritable` | read fine, the database write did not — also absent |

`unreadable` and `unwritable` exist so a reader holding only the archive can
see the census is incomplete without querying anything. `finished_at` absent
means the run was interrupted and the counts are wherever it got to.

### `<chain>/<agent_id>.json`

One agent: `run_id`, `chain`, `agent_id`, `token_id`, `owner`, `agent_uri`,
`block_number`, the `checks` array (every rung, its status, and its evidence),
and a summary of what the fetch saw — `http_status`, `content_type`,
`body_bytes`, `body_sha256`, `final_url`.

`checker_commit` and `spec_commit` are repeated in every agent file on
purpose: a single file handed to somebody has to be self-describing without
the manifest beside it.

**The document bodies are not in the export.** They can be up to 1 MiB each
and live only in `http_archive.body` in the database. `body_sha256` is there,
so you can verify a document you fetch yourself against what we fetched.

## Two things in the data that are about people

Stated here rather than discovered by whoever loads a dump.

- **Wallet addresses** (`owner`, and `minter` from schema 6 on) are on-chain
  values. Pseudonymous, already published by every block explorer, and
  reproduced as the chain records them.
- **Email addresses.** 351 agents across the four-chain census published an
  email address as a `services[].endpoint` value. Rung 6's evidence records
  every declared endpoint verbatim, so those addresses are in these archives.
  They were published on a public chain by their own registrants as contact
  points. We reproduce them as recorded and do not redact — but publishing
  makes them collectable at scale in a way an individual on-chain value is
  not, so if you are one of those registrants and want yours excluded, email
  <probes@agentcount.ai> and it will be redacted from future exports.

Nothing else about any person is in here. Subscriber addresses, if this
project ever has any, are never exported — see `scripts/publish-run.sh`, which
publishes a fixed set of files rather than whatever it finds.

## Verifying an archive

```sh
run=cfbfcc01-fdaf-409f-9bed-abf706d865c7
curl -LO https://storage.googleapis.com/agentcount-data/runs/$run.tar.zst
curl -LO https://storage.googleapis.com/agentcount-data/runs/$run.tar.zst.sha256
shasum -a 256 -c $run.tar.zst.sha256
tar --zstd -xf $run.tar.zst
```

The same hash is committed to
[`published-runs.json`](published-runs.json) in this repository, so the git
history attests the archives and not only the numbers derived from them. An
archive that verifies against a hash in a commit predating any dispute is
evidence; one that only matches a file on a server we control is not.

## Loading an archive into Postgres

An archive is also how you get a real census into a local database without
sweeping a chain: `import-run`, in this repository, is the exact inverse of
the export — it reads `manifest.json` and the per-agent files and writes the
`runs`, `agent_snapshots`, `check_results` and `http_archive` rows they came
from. `DATABASE_URL=… cargo run -p sweeper --bin import-run -- <archive.tar.zst>`
after `sqlx migrate run`; the README's "Local data without sweeping" section
has the full steps. The one asymmetry is the one stated above: bodies are not
in archives, so an imported `http_archive` row carries the summary columns
(status, content-type, size, `body_sha256`, final URL) and never the bytes.

## Re-deriving a headline number

The July 2026 census reports that **61.0% of valid registration documents
declare no way to reach the agent**. That is rung 4's `services_absent_or_empty`
over the agents that reached rung 4, on Base. From the archive alone:

```sh
run=cfbfcc01-fdaf-409f-9bed-abf706d865c7
tar --zstd -xf $run.tar.zst

# `find … -exec cat {} +` rather than a glob: this run has 60,097 agent files
# and `jq … $run/base/*.json` dies with "argument list too long". Streaming
# also avoids holding all 60,097 parsed objects in memory at once.
find "$run/base" -name '*.json' -exec cat {} + | jq -r '
  (.checks[] | select(.rung == 4)) as $r4
  # Denominator: agents whose rung 4 was actually answered. A `skipped` rung 4
  # means an earlier rung stopped it, and it never had a services field to
  # judge — counting those would be dividing by the wrong population.
  | select($r4.status != "skipped")
  | if ($r4.evidence.services_status // "") | (. == "absent" or . == "empty")
    then "no_services" else "has_services" end
' | sort | uniq -c
```

```
11704 has_services
18327 no_services
```

18,327 / (18,327 + 11,704) = **61.0269%**, which is the published
`61.026938829875796%` to every digit the report prints. The same figure comes
from `GET /api/runs/<run_id>/findings` as `services_absent_or_empty`.

If your result disagrees, the published figure is wrong and we want to know.
Open an issue with the run id and the commands you ran.

## Archives are census runs only — never the registration tail

An archive is one run: the agents a pinned block held, and the seven answers
that sweep produced for each of them. Nothing else is in it.

In particular, the **registration tail** is not. Between sweeps a poller
records agents minted since the last census — id, owner, declared URI, and the
block it read them at — so a brand-new agent can still be found and linked to
on the site (`/api/tail`). Those rows carry no check results, belong to no run,
and are in a table (`registration_tail`, migration 0018) with no `run_id` and
no foreign key to `runs`. The export walks a run's rows, so there is no path by
which a tail row could reach an archive; and `scripts/publish-run.sh` publishes
a fixed list of paths, so a new table cannot start being uploaded by accident.

This matters if you are re-deriving a number. A run's `agent_count` is the
population at that run's pinned block, and it will be smaller than what the
chain holds today — that difference is real, and it is the point of a pinned
census rather than a rolling one. If you want the current gap, ask
`GET /api/tail/summary`, which reports per chain how many agents the tail has
seen that no census has checked. Adding that count to a run's population
produces a number that describes no moment in time and is not a figure this
project publishes.

## A known defect in the 2026-08 archives

Every run in the 2026-08 census records **`checker_commit: unknown`**.

The stamp is produced at build time by `git rev-parse HEAD`. The weekly job's
image is built by Cloud Build from an uploaded tarball with no `.git` in it, so
that command failed and the build script fell back to its honest placeholder —
honest, but useless: `checker_commit` is what lets you fetch the exact code
that produced a result, and "unknown" means you cannot.

Fixed for every run after 2026-08-05: the commit is passed into the image build
explicitly, and a build from a dirty tree is stamped `-dirty` rather than
claiming to be a commit.

**The affected archives are not being reissued.** They are immutable, which is
the whole point of publishing them at a permanent URL, and quietly replacing
bytes to correct a metadata field would break the only guarantee that makes a
hash worth committing. What can be recovered is recovered here instead: those
runs were produced by the code at
[`3235667`](https://github.com/AgentCount/agentcount/commit/3235667527c2e4e11a672f64d3358133d024a808)
or an immediate ancestor — checker `0.6.0`, schema `7` — and their
`checker_version` and `spec_commit` fields are correct and unaffected.

## Schema versions

`schema_version` in the manifest says which evidence contract a run was
written under. It is not decoration: rung 4's evidence changed shape at
version 2, and rung 6 did not exist before version 7. A tool that reads
archives across versions has to branch on it.

| version | what changed |
|---|---|
| 1 | the original ladder |
| 2 | rung 4 evidence: `must_violations`/`should_gaps`/`may_gaps`, `services_status` |
| 3 | `unclaimed` status; rung 7 renamed `attested` and ungated |
| 4 | rung 2 evidence: `data_uri_variant`, `data_uri_algorithm` |
| 5 | rung 2 evidence: `gateway_attempts` |
| 6 | `minter`, `registration_tx_hash`, `registration_block`; rung 1 `tx_hash` populated |
| 7 | rung 6 (`live`) ships; `unprobeable` status; `endpoint_probes` |
| 8 | `refused` status on rungs 2 and 6; rung 2 evidence: `retry_after`; rung 6 evidence: `endpoints_refused` |

Full detail for every one of these is in
[`CHANGELOG-METHODOLOGY.md`](CHANGELOG-METHODOLOGY.md).

### Version 8 is the one that moved existing rows

Every earlier bump was additive: a new evidence field, or a status for a case
nothing had ever been written for. **Version 8 renamed answers that already
existed.** HTTP 429/503/401/402/407, and a `robots.txt` we could not get
permission from, were `fail` (the agent's word) or `error` (ours); they are now
`refused`. Nothing was re-probed and no agent's behaviour was re-assessed — the
new word is derived from the `http_status` and `reason` each row already
carried, and no row moves into or out of `pass` in either direction.

The database has been re-judged so the published series says one thing. **The
archives already published have not been reissued**, for the same
reason as the `checker_commit` defect above: immutable bytes at a permanent URL
are the guarantee. So an archive stamped schema 7 or earlier carries the old
words, and the mapping to schema 8 is mechanical and total:

| in a schema ≤ 7 archive | in schema 8 |
|---|---|
| rung 2 `fail`, `evidence.http_status` one of 401/402/407/429/503 | `refused` |
| rung 2 `error`, `evidence.reason` starting `robots_disallowed` or `robots_unavailable` | `refused` |
| rung 6 `fail`, some endpoint 401/407/429/503 and none live or definitely not-live | `refused` |
| rung 6 `error`, every endpoint's error a `robots_*` one | `refused` |
| anything else | unchanged |

The 2026-08-06 changelog entry has the measured counts per run.

## What is NOT published

- **Subscriber email addresses**, if this project ever collects any.
- **RPC endpoint URLs**, which carry API keys.
- **Database credentials.**
- **Document bodies** — see above; the hashes are published instead.
- **Registration-tail rows** — see above; they are not census data, so they
  are not in any run's archive.

The publish script writes an explicit list of paths rather than uploading a
directory, so adding a new table or a new file to the working tree cannot
silently start publishing it.
