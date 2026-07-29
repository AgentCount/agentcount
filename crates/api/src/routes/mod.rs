//! The route handlers, grouped by what they serve.
//!
//! Splitting handlers into files by resource keeps `main.rs`'s router readable
//! and each file focused:
//!   * [`runs`] — the run list, and the shared "latest completed run" lookup
//!     other handlers use to fill in a missing `run=`.
//!   * [`rates`] — the headline output: per-rung status counts for one run.
//!   * [`findings`] — the same population, cross-cut into the handful of
//!     numbers the census leads with. Still counts, still no score.
//!   * [`agents`] — the directory and single-agent detail.
//!   * [`methodology`] — the provenance constants and rung-4 field list, as data.
//!
//! `pub mod` (rather than plain `mod`) because `main.rs` needs to name these
//! handlers when building the router, e.g. `routes::agents::list`.

pub mod agents;
pub mod findings;
pub mod methodology;
pub mod rates;
pub mod runs;
