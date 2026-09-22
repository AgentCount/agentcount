# Working on AgentCount

Read `CONTRIBUTING.md` first — particularly **“Before you ship a change to the
sweep”**, which is the checklist this file exists to make sure you actually
reach. What follows is only what an agent needs that a human contributor picks
up by being here.

## What this project is

Two instruments, deliberately kept apart. The **Registration Census** counts
agents registered under ERC-8004 across eleven chains; the **Seller Census**
(METHODOLOGY §10) asks whether the sellers advertising paid resources over
x402 answer, quote a payable price, and have ever been paid. They measure
different populations and must never be blended into one figure.

The product is not the code. It is the claim that every published number can
be recomputed from an archived run, and that the census never reports its own
failures as the world's behaviour. Most of the hard-won rules in this
repository exist to protect one of those two.

## The rules that keep being relearned

**Absence is never a status.** `error` is ours, `refused` is an origin
declining us, `unprobed` is a question we chose not to ask, `skipped` is a
prerequisite that did not pass. None is a failure of the thing being measured,
and none belongs in a denominator. A rate over nobody is undefined, not 0%.

**A rung nobody asked is not a rung that scored zero.** Rung 4 of the Seller
Census has never run. Anything that renders it as 0% is publishing a ruinous
claim about real businesses.

**Method before numbers.** A change to what gets measured is a dated entry in
`CHANGELOG-METHODOLOGY.md`, written before the figure it produces.

## Operational shape

- GCP project `vippalo`, region `europe-north1`.
- Four Cloud Run jobs on a four-day rota: `agentcount-sweep` (nine chains,
  Mon), `agentcount-sweep-bsc` (Tue), `agentcount-sweep-base` (Wed),
  `agentcount-sellers` (Thu). All share one image, `sweep:latest`, by tag.
- Two dead man's switches, and they must stay separate: the census pings
  `HEARTBEAT_URL`, the Seller Census pings `SELLER_HEARTBEAT_URL`. Pointing
  both at one monitor lets a healthy Thursday vouch for a Monday that never
  ran — which is how a two-week outage went unseen.
- The database is reachable through the Cloud SQL proxy on port 5433.

## Habits that would have saved a week each

**Assert that your edit applied.** A scripted replace that matches nothing
reports success. That is how the sweep job came to carry four RPC secrets for
nine chains, and how `SELLER_HEARTBEAT_URL` was wired into a script that never
received it. Check the file afterwards, not the command's exit code.

**Verify the thing, not a proxy for it.** See CONTRIBUTING.md; this is the
single most expensive habit in this repository's history.

**Simulation validates arithmetic and hides assumptions.** A watchdog replayed
against real timings looked correct and was measuring the wrong phase. If a
change decides when to stop work, it ships in observe mode first.
