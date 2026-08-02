# AgentCount

An independent conformance and census layer for
[ERC-8004](spec/SOURCE.md) ("Trustless Agents").

AgentCount enumerates every agent registered in an ERC-8004 Identity Registry,
reads each one's on-chain state at a pinned block, fetches the off-chain
document it points at, and checks whether its declared endpoints and reputation
entries hold up. The result for each agent is **seven questions and the
evidence behind each one**.

**It counts. It does not score.** There is no aggregate, no grade, no tier, no
ranking — not even a "5 of 7 rungs passed" tally. A 0–100 number needs weights,
and weights need ground truth to calibrate against; none exists for this
population, so any weighting would be a design decision dressed as a
measurement. Population base rates *are* published, because those are reached
by counting. What is never published is a single number standing in for one
agent.

Every claim carries something a reader can re-check: a transaction hash, an
HTTP status, a JSON diff, a block number. Where something could not be checked,
that is recorded as such rather than guessed.

## Scale

The 2026-07-30 census: four chains, each pinned to a block — 354,858 agents.
The current chain set is whatever [`published-runs.json`](published-runs.json)
lists; this table is a dated snapshot, not the live scope.

| chain | agents | pinned block |
|---|---:|---:|
| BNB Chain | 244,208 | 112,874,357 |
| Base | 60,097 | 49,262,617 |
| Ethereum mainnet | 40,806 | 25,640,407 |
| Celo | 9,747 | 73,448,013 |

Reports are in [`docs/reports/`](docs/reports/); the working behind individual
findings is in [`analysis/`](analysis/).

## The seven rungs

The ladder is ordered — each rung asks something that only makes sense once the
rung below it passed — except rung 7, which sits on its own track, because
reputation is readable whether or not a document ever resolved.

| # | name | question |
|---|---|---|
| 1 | `registered` | Does this agent id exist in the Identity Registry as a currently-held ERC-721 token? |
| 2 | `resolvable` | Does `tokenURI()` return a URI, and does fetching it return a successful HTTP response? |
| 3 | `parseable` | Does the fetched body parse as valid JSON? |
| 4 | `conformant` | Does the document carry the fields the spec requires, at MUST / SHOULD / MAY severity? |
| 5 | `bound` | Does the document name the agent id, registry and chain it claims to belong to? |
| 6 | `live` | Does every declared service endpoint respond? **Not implemented — absent from every result, never a failure.** |
| 7 | `attested` | Does the Reputation Registry hold feedback for this agent from someone other than its owner? |

Each rung returns one of: `pass`, `fail`, `skipped`, `error`, `unclaimed`
(rung 5 only) — or, for an unimplemented rung, **absent**. These are distinct
states and are never collapsed into one another. "We did not ask" and "we asked
and got no answer" are different claims, and the schema keeps them different.

[`METHODOLOGY.md`](METHODOLOGY.md) is the authority on what each rung checks,
what evidence backs it, and what it explicitly does not mean. It was published
before any findings existed, so the method can be checked before the numbers.
[`CHANGELOG-METHODOLOGY.md`](CHANGELOG-METHODOLOGY.md) records every change to
what is measured — including the several times an earlier version of the method
was wrong, and what each correction moved.

## Layout

Six crates in one Cargo workspace. Postgres is the only shared state; the
binaries never talk to each other directly, so a failure in one stage cannot
corrupt another.

| crate | job |
|---|---|
| `crates/chain` | typed reads of the Identity and Reputation registries |
| `crates/indexer` | follows registry events per chain |
| `crates/probe` | HTTP fetching: `robots.txt`, redirects, SSRF guard, per-host caps |
| `crates/checks` | **the only place a verdict is formed** — pure functions, no I/O |
| `crates/sweeper` | runs the ladder over one chain, once, and writes a run |
| `crates/api` | the JSON API the frontend reads |
| `migrations/` | sqlx migrations — the schema |
| `scripts/seed_chains.sql` | which chains, which registry addresses |

`crates/checks` is deliberately pure: plain data in, a status plus evidence
out, with no network and no database. That is what makes a verdict
re-derivable from an archived run without re-reading the chain, and CI enforces
it as a named job.

The frontend that renders `crates/api` lives in
[`AgentCount/agentcount-web`](https://github.com/AgentCount/agentcount-web).
Display issues get filed there; this repo is canonical for anything that can
move a status.

## Running a census

Needs PostgreSQL 16+ and an RPC endpoint for the target chain.

```sh
createdb agentcount
export DATABASE_URL=postgres://localhost:5432/agentcount
cargo install sqlx-cli
sqlx migrate run

psql "$DATABASE_URL" -f scripts/seed_chains.sql

export RPC_URL_BASE=...          # per chain: RPC_URL_<CHAIN>, uppercased
cargo run --release -p sweeper -- base
```

The sweep prints a run id. Every row it writes is stamped with
`schema_version`, `checker_version`, `checker_commit` and `spec_commit`, plus a
literal `rerun_command` — so any published number traces to the exact code and
spec text that produced it. **Runs are immutable:** a newer answer is a new
run, never an edit to an old one.

`SWEEP_RESUME=<run_id>` continues an interrupted sweep without re-reading what
it already wrote.

### Environment

| variable | used by | what |
|---|---|---|
| `DATABASE_URL` | all | Postgres connection string |
| `RPC_URL_<CHAIN>` | indexer, sweeper | JSON-RPC endpoint per enabled chain |
| `RPC_CONCURRENCY` | sweeper | concurrent chain reads (default 9) |
| `PROBE_CONCURRENCY` | sweeper | concurrent HTTP fetches |
| `SWEEP_RESUME` | sweeper | run id to continue |
| `RUST_LOG` | all | log verbosity, e.g. `sweeper=info` |

Bring your own RPC provider. Note that going *over* a provider's rate limit is
slower than staying under it — 429s trigger retries that spend more quota than
they save.

## Local data without sweeping

You do not need an RPC endpoint or a multi-hour sweep to work on the API or
the checks: every published run is a free archive ([`DATA.md`](DATA.md)), and
`import-run` loads one into your local database in under a minute.

```sh
createdb agentcount
export DATABASE_URL=postgres://localhost:5432/agentcount
cargo install sqlx-cli
sqlx migrate run

# celo, the smallest published run: 4.1 MB, 9,747 agents
run=7833fc49-a5b7-477b-99ce-946f650f0064
curl -LO https://data.agentcount.ai/runs/$run.tar.zst
cargo run -p sweeper --bin import-run -- $run.tar.zst

cargo run -p api      # GET /api/runs now serves a real census
```

`import-run` takes an archive or an already-extracted directory, refuses a
run id that is already in the database unless `--replace` is passed (delete
and re-import, one transaction), and seeds a minimal `chains` row when the
manifest names a chain your database has not seen — run
`scripts/seed_chains.sql` whenever you want the full chain configuration.
Extraction shells out to `tar --zstd` (falling back to `zstd -dc | tar`), so
have zstd installed. One caveat, inherited from the archives themselves:
document **bodies** are not in exports, so `http_archive` gets summary
columns only — verdicts and evidence import completely, re-judging from raw
bytes needs a sweep of your own.

## Reproducing a published figure

Most figures need neither this codebase nor our database. Event-derived
results — validation registry usage, feedback values, endpoint kinds — are
reproducible with `cast` alone, and each document in [`analysis/`](analysis/)
carries the exact commands, the pinned blocks and the date it was queried.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). One rule is worth stating up front:
**a change to check semantics needs an issue and a written rationale before a
pull request.** Anyone whose agent fails a check has an incentive to argue the
check is wrong; that policy keeps the argument in the open, attached to
fixtures and a before/after count.

Security reports go privately to `probes@agentcount.ai` — see
[`SECURITY.md`](SECURITY.md).

## Licence

[Apache-2.0](LICENSE). The code is forkable; the **AgentCount** name and logo
are not — see [`TRADEMARK.md`](TRADEMARK.md).
