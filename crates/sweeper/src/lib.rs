//! The parts of the sweeper that more than one binary needs.
//!
//! This crate produces two executables against the same database:
//!
//! * **`sweeper`** (`src/main.rs`) — the census itself. Pins a block, reads
//!   every agent, fetches its registration document, and answers rungs 1-5
//!   and 7.
//! * **`liveness`** (`src/bin/liveness.rs`) — rung 6. A second pass over a
//!   run that already exists, probing the service endpoints those documents
//!   declared.
//!
//! Rung 6 is a separate pass rather than a stage of the main pipeline for a
//! reason that is about the unit of work, not about convenience: the main
//! sweep's unit is an agent, and rung 6's is a URL. Deduplicating endpoints
//! and applying a per-host budget both need the WHOLE population in hand
//! before the first request goes out, which a per-agent pipeline cannot
//! provide. Four hosts carry 59.2% of every declared endpoint in the census;
//! probing them agent-by-agent would send 26,273 requests to one server to
//! learn one fact.
//!
//! Splitting them also means rung 6 can be re-run against an existing run
//! without re-sweeping the chain — the probe is the slow, rate-limited,
//! interruptible half, and it should not be able to cost a chain read.
//!
//! A third binary is a second pass of the same shape, answering a question
//! that is not a rung at all:
//!
//! * **`payments`** (`src/bin/payments.rs`) — who has ever been paid, pinned to
//!   the run's block. It reads `getAgentWallet` and the archived documents,
//!   builds the attribution map, scans two stablecoins' `Transfer` logs, and
//!   writes `payment_targets` / `payment_scans` / `payments`. It never touches
//!   `check_results`: "was this agent paid" is judged against no clause of
//!   ERC-8004, and a row in the rungs' table would make it the eighth rung by
//!   placement. The rule it enforces is `crates/payments`' and nothing else's.
//!
//! A fourth binary runs on a different clock and answers a different question:
//!
//! * **`tail`** (`src/bin/tail.rs`) — the continuous registration tail. NOT a
//!   census pass. It discovers ids minted since the last sweep, so an agent
//!   registered five minutes ago can still be found and linked to, and it
//!   records only what an on-chain read gives cheaply: owner, URI, block. No
//!   document is fetched and no rung is answered. See [`tail`]'s module doc
//!   for why its table is unreachable from every census aggregate by
//!   construction rather than by convention.
//!
//! `store` is here because both binaries write to the same tables and must
//! agree exactly about how. `export` stays private to the sweeper binary: it
//! writes the `data/<run_id>/` files, which only a full sweep produces.

pub mod delta;
pub mod store;
pub mod tail;
