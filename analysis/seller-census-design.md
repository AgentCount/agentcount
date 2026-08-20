# Seller Census: design — who is actually selling in the x402 economy, and does it hold up

**Design only. No crate code, no migration, no probe, no purchase. Nothing
here is a number, and no number may be produced from this design until it has
survived review and its method is in `METHODOLOGY.md` — the same
methodology-before-numbers order every instrument in this project follows.**

This is Instrument 02. Instrument 01 (the ERC-8004 registration census) asks
whether the agents everyone counts are real; this one asks whether the
economy everyone cites is real. The DNA is identical — enumerate a
population nobody has enumerated, ask each member the same yes/no questions,
publish per-member evidence and population rates, pin everything, score
nothing.

---

## 1. The unit: what a "seller" is

A **seller** is a deduped **(payTo, host)** pair:

* **payTo** — the payment-receiving address a resource's `402` payment
  requirements name, normalized per network (EVM addresses lowercased;
  Solana addresses verbatim, they are case-sensitive).
* **host** — the full lowercase host of the resource URL (port stripped when
  it is the scheme default, IDN in punycode). The **full** host, not the
  registrable domain: `api.example.com` and `example.com` are different
  services with different operators' hands on them, and collapsing them
  would manufacture consistency that was never claimed.

The same payTo behind two hosts is **two sellers**; the same host quoting two
payTos is **two sellers**. Either collapse would blend measurements the same
way blending two runs does in the census (README, "run-scoped, always").
Groupings — "these 40 sellers share one payTo", which is how a facilitator
or an aggregator appears — are published as a **finding over the
population**, exactly as the identity-role audit treats shared payment
contracts, never as a merge of the units.

A seller has **resources** (the individual priced URLs below its host that
name its payTo). Resources are children of the seller: evidence is recorded
per resource, and a seller's answer at a rung is derived from its resources
by a stated aggregation rule (§3, per rung). The seller count and the
resource count are both published; neither stands in for the other.

## 2. Enumeration: the catalogs

Sellers are enumerated from every catalog, because every catalog is partial
and nobody publishes the union:

| # | Catalog | What it is |
|---|---------|------------|
| C1 | CDP Bazaar (`/discovery/resources`) | Coinbase's x402 discovery index |
| C2 | x402scan directory | The largest independent index (Apache-2.0) |
| C3 | thirdweb Nexus | Framework-side listing |
| C4 | B402 | Independent directory |
| C5 | x402 Atlas | Independent directory |
| C6 | `.well-known` / OpenAPI `x-payment-info` conventions | Self-declared, crawled only from hosts already named by C1–C5 |

Rules that make this a census rather than a scrape:

* **Snapshot, hash, archive.** Every sweep archives each catalog's raw
  response bytes, hash-committed like run archives (`DATA.md` mechanics).
  Catalogs are mutable and off-chain; the archive is what makes "listed on
  2026-W38" a checkable claim a year later.
* **Provenance per seller.** Which catalogs list a seller is evidence on the
  seller (`catalogs: [C1, C2]`), which makes the cross-reference — the union
  index nobody else has — a stored fact rather than a marketing claim.
* **The catalog list is versioned.** Adding or removing a catalog changes
  the population and is a methodology-changelog event, exactly as enabling a
  chain is for Instrument 01. A seller that disappears because its only
  catalog was removed is a method change, not churn — the delta rules (§6)
  must not be able to say otherwise.
* **C6 never introduces hosts.** Crawling the open web for payment
  conventions has no stopping rule and no defensible population claim. C6
  only enriches hosts that C1–C5 already named.

## 3. The rungs

Statuses reuse the census vocabulary verbatim — `pass`, `fail`, `error`
(OURS, never the seller's), `refused` (the origin declined us), `skipped`
(a prerequisite did not pass) — plus one new word this instrument needs:
**`unprobed`** (we chose not to ask; §4 says when and why). `error`,
`refused` and `unprobed` are never publishable as a seller's failure, and
the delta rules exclude them from churn from day one (§6) — the 19,983 and
4,479 incidents are baked in here, not waiting to be rediscovered.

robots.txt binds every request this instrument makes — catalog enrichment,
rung-2 reachability, the rung-3/4 protocol probe, rung-7 fetches — with no
carve-out for the 402 handshake (§8.1). A host that disallows us is
`refused`, stated as such.

Unlike Instrument 01 this is **not a strict ladder** — settlement does not
require delivery — so each rung names its prerequisites explicitly and
`skipped` refers to them:

| # | Rung | Question | Prerequisite | Seller-level aggregation |
|---|------|----------|--------------|--------------------------|
| 1 | `listed` | Does ≥1 catalog list it? | — | By construction `pass` (the population IS the listed); the evidence is which catalogs, and since when. |
| 2 | `reachable` | Does the host answer at all? | 1 | One probe per host. Any HTTP response — including 4xx/5xx — is `pass`: the question is existence, not health. |
| 3 | `quotes` | Does a resource return a spec-valid 402 — parseable payment requirements naming a scheme, network, amount, asset and the payTo that defines this seller? | 2 | `pass` if ≥1 resource quotes validly; per-resource results stored. Validated against a **pinned x402 spec commit**, like `spec/` pins ERC-8004. |
| 4 | `delivers` | Given a real payment, does it serve the resource? | 3 | The shopper (§4) probes exactly one resource: the cheapest at-or-under cap. `pass` = resource delivered; `fail` = payment settled, resource not delivered (the strongest finding this instrument can produce, and the evidence bar is correspondingly §4's highest); `unprobed` = every resource priced above cap. |
| 5 | `receipted` | Does it sign offers/receipts per the x402 receipts extension? | 4 attempted | Evaluated on the same purchase — no extra spend. Expected to be nearly universally `fail` at first; that is the finding, not a defect of the design. **Held out of the first locked method (§8.5); the number stays reserved.** |
| 6 | `settled` | Does the payTo have on-chain settlement history, facilitator-agnostic, at a pinned block? | 1 | Chain-scoped scan (the `crates/payments` pattern: pinned block, stated basis, stated exclusions). Evidence: first/last settlement, count, **distinct payers**. Our own shopper payments are excluded by the published wallet (§4). |
| 7 | `consistent` | Does what the catalogs claim (price, description, schema) match what the endpoint quotes? | 1 ∧ 3 | Compared field-by-field between the archived catalog snapshot and the archived 402 body. Divergence is recorded per field. A seller in two disagreeing catalogs is judged against each, and the disagreement is itself evidence. |

What is deliberately **not** measured: revenue, volume in dollars, uptime,
latency, quality of the delivered resource. The first two belong to the
Reconciliation instrument with their own method; the rest are scores wearing
a trench coat.

## 4. The mystery shopper

Rung 4 is the number that does not exist anywhere: **the verified delivery
rate of the agent economy**. Paying real sellers real money, on a schedule,
is only audit-grade under pre-registered rules:

* **The wallet is published before the first purchase.** The shopper's
  address (per network) goes into `METHODOLOGY.md` in the PR that locks this
  method — before any purchase, so that (a) anyone measuring x402 volume can
  exclude our probes, (b) nobody can accuse this project of inflating the
  volume it audits, and (c) our own rung-6 scans can exclude it
  mechanically. A wallet that had to be rotated is a changelog event.
* **Price cap.** Only resources quoting ≤ **$0.10** (in the quoted asset's
  face value) are purchased. Everything above is `unprobed`, and the
  unprobed count is published beside the delivery rate it qualifies —
  "delivered 61% (of the 83% of sellers under cap)" is the honest sentence
  shape.
* **One purchase per seller per sweep.** The cheapest at-or-under-cap
  resource. No retries within a sweep: a payment that settled and a resource
  that did not arrive is the measurement, not a flake to be smoothed over.
* **Politeness is a launch-blocking requirement, not a setting.** Per-host
  concurrency 1; per-host budget of probed sellers per sweep (500, the
  census's number, until evidence says otherwise); `Retry-After` honoured
  everywhere with the census's exact semantics — a 429/503 is `refused`, is
  excluded from churn, and is OUR signal to slow down. This project already
  manufactured 19,962 of its own headline metric once; the shopper
  multiplies that risk by adding money.
* **Delivered content is evidenced, not archived.** The census archives
  registration documents because agents publish them to be read. A purchased
  resource is a product. We store its hash, size, content-type,
  schema-validity against the quote, and HTTP metadata — enough to make
  `delivers` recomputable-in-principle and disputable-in-fact — and never
  the body, never republished.
* **Spend is published.** Per sweep: total spent, per-network, per-outcome
  (delivered / not delivered / payment failed). At ~5,000 under-cap sellers
  × ~$0.05 that is ~$250/sweep — the cheapest number this project will ever
  buy.
* **Failure evidence is the strongest evidence.** A `fail` at rung 4 states
  a merchant took payment and did not deliver. The row must carry the
  settlement proof (tx hash / authorization), the full response, and
  timestamps — the Celo-piece rule: what the evidence shows, never intent.

## 5. Pinning and reproducibility

* Catalog snapshots: archived bytes + hashes, per sweep (§2).
* Rung 6: pinned block per network, stated scan basis and exclusions —
  `crates/payments`' discipline, generalized from "was this agent ever
  paid" to "has this payTo ever settled".
* Rungs 2–5, 7: HTTP facts, timestamped not block-pinned — the same honesty
  the census applies to rung 6 liveness (`METHODOLOGY.md` §7: a probe is a
  fact about that moment).
* Every published rate carries its denominator, and every rung's population
  is "sellers asked", never "sellers total" — absence of an answer is
  absence, not a status (the 404-not-zeros rule, everywhere).

## 6. Deltas, from day one

The weekly unit of change ships with the instrument, not four migrations
later: `appeared` / `disappeared` (population, per catalog and net),
`went_dark` / `came_back` (rung 2, with `refused`/`error` transitions
excluded from the headline and totalled visibly — the `NOT_CHURN` rule as
extended 2026-08-18), and the delivery-rate pair movement. The confound rule
travels too: any pair spanning a method change (spec commit, catalog list,
cap, checker) says so on the row.

## 7. What this instrument will not do

* **No scores.** Not a trust score, not a ranking, not a badge.
* **Nothing publishable is purchasable.** No paid listings, no paid
  verification, no seller-paid anything that touches a published fact
  (GOVERNANCE.md already says this; it is restated here because sellers
  have money and will ask).
* **No intent.** "Took payment, did not deliver" is publishable evidence;
  "scam" is not a word this census has a rung for.

## 8. Decisions — the review of 2026-08-20

The five questions this design left open were decided with the maintainer on
2026-08-20. They are recorded here as rules, with the road not taken, so the
next reader does not relitigate them without new evidence.

1. **robots.txt binds everything, including the 402 handshake.** One rule,
   no carve-outs: C6 crawling, rung 7 fetches, rung 2, and the rung-3/4
   protocol probe all honour robots.txt, and a host that disallows us is
   `refused`, stated as such. The carve-out ("a 402 handshake is the
   protocol's designed use, not crawling") was considered and declined: it
   costs some coverage, but the auditor's neutrality armour stays seamless —
   nobody can accuse this census of ignoring a no.
2. **The cap is $0.10 face value in stablecoin quotes; anything else is
   `unprobed` with reason `unpriced`.** No conversion rule and no pinned
   price source in v1 — inventing a pricing methodology to spend ten cents
   is complexity before evidence. Revisit when a real population of
   non-stablecoin quotes exists.
3. **Sweep 1 is Base (USDC) only.** Complete coverage of one network beats
   partial claims about two. Solana follows as a stated expansion with its
   own changelog entry — the enable-a-chain playbook from Instrument 01.
4. **Identity stays `(payTo, host)`; rotation is a finding.** A seller that
   rotates its payTo weekly will read as churn, and that is accepted: the
   churn is real information (a host whose payment address never stabilises
   is a pattern worth publishing), where host-only identity would blend
   genuinely distinct sellers behind shared hosts — the exact
   compares-an-agent-to-itself failure the census forbids.
5. **Rung 5 (`receipted`) is held out of the first locked method.** The
   receipts extension is still moving, and a rung that re-judges the
   population on every upstream edit is method churn in the
   credibility-critical launch window. The rung number stays reserved and
   the design above stands; it enters the method as a stated expansion once
   the extension stabilises — pinned to a spec commit, like rung 3.

---

*Next artifact after this survives review: the METHODOLOGY section (the
method, locked), then the crate skeleton (`crates/sellers`), catalog crawler
+ rungs 1–3, and only then the shopper — wallet published first.*
