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
//! `store` is here because both binaries write to the same tables and must
//! agree exactly about how. `export` stays private to the sweeper binary: it
//! writes the `data/<run_id>/` files, which only a full sweep produces.

pub mod store;
