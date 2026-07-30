# Security policy

## Reporting

**Report privately to `probes@agentcount.ai`.** Please do not open a public
issue for a vulnerability.

Include what you can: what you did, what happened, and — if it is safe to — how
to reproduce it. A rough report beats no report; you do not need a proof of
concept.

Expect an acknowledgement within a few days. This is a small project with one
maintainer, so please read that as a good-faith intention rather than a
guaranteed SLA.

## Credentials in this repository

There are none, and that was verified rather than assumed. Before this
repository was made public, **every commit in its history was audited** for
committed `.env` files, RPC provider keys, database URLs and JWT-shaped
strings. Nothing was found: the only connection strings that have ever appeared
are `localhost` and an obvious `user:pass@host/db` placeholder in
documentation.

If you find something that audit missed, that is exactly the kind of report
this document is for.

## Scope

The things most worth attacking here, roughly in order:

- **The SSRF guard** (`crates/probe/src/netguard.rs`). Every agent-supplied URI
  is hostile input by construction: anyone can register an agent and point its
  `tokenURI` anywhere. Requests are restricted to public IP addresses, and
  **every redirect hop is re-validated**, not just the first request. A way to
  reach a private, loopback, link-local or cloud-metadata address — including
  via DNS rebinding, a redirect chain, or an IPFS gateway rewrite — is the
  highest-severity bug in this codebase.
- **The frontend's URL handling.** Agent-supplied strings are rendered as
  links. `data:`, `javascript:`, `vbscript:`, `file:` and `blob:` are refused
  by a scheme allowlist, and third-party links carry
  `rel="noopener noreferrer nofollow ugc"`. A bypass is in scope.
- **SQL injection or unbounded resource use in the API** (`crates/api`).
- **Anything that lets a third party alter a stored run**, which would break
  the immutability the whole method rests on.

## Explicitly not vulnerabilities

- **A check you disagree with.** That is [`CONTRIBUTING.md`](CONTRIBUTING.md)'s
  process, not a security report.
- **A published number you believe is wrong.** Genuinely welcome — open a
  public issue with the run id and agent id. This project has retracted its own
  findings before publication more than once.
- Volume or timing of our probe traffic. Email the same address and we will
  suppress a host or agent from future sweeps; see `METHODOLOGY.md` §6.

## Disclosure

Coordinated. Tell us, give us a reasonable window to fix it, then publish
whatever you like — including that we were slow, if we were. Fixes to check
semantics that result from a report still go through the public process in
`CONTRIBUTING.md`, because a check that changed for unexplained reasons is its
own kind of problem.
