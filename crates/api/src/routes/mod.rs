//! The route handlers, grouped by what they serve.
//!
//! Splitting handlers into files by resource keeps `main.rs`'s router readable
//! and each file focused:
//!   * [`runs`] — the run list, and the shared "latest completed run" lookup
//!     other handlers use to fill in a missing `run=`.
//!   * [`rates`] — the headline output: per-rung status counts for one run.
//!   * [`findings`] — the same population, cross-cut into the handful of
//!     numbers the census leads with. Still counts, still no score.
//!   * [`deltas`] — what changed between one run and the previous one, read
//!     back from the row the `delta` binary wrote at sweep time. The one
//!     legitimate two-run figure — see its module doc for why.
//!   * [`agents`] — the directory and single-agent detail.
//!   * [`search`] — one `q` across several caller-named runs, grouped per
//!     run. The only endpoint that touches more than one run, and it still
//!     never blends their rows.
//!   * [`sellers`] — the OTHER instrument (METHODOLOGY §10): the seller runs
//!     and their per-rung counts. Deliberately its own URLs and its own
//!     types rather than a `network` switch on the four modules above —
//!     these are different populations, and the one mistake that would
//!     discredit both censuses is a figure that blends them.
//!   * [`tail`] — the one module that serves rows belonging to NO run: agents
//!     the chain has that no census has checked yet. Its response shape shares
//!     almost nothing with a census result, on purpose — read its module doc
//!     before adding a field to it.
//!   * [`spot_check`] — one agent, checked on demand, right now. The only
//!     endpoint that reads a chain or sends a request to a third party, and
//!     therefore the only one whose module doc is mostly about restraint:
//!     two rate limits, an existence guard, and a written argument for why
//!     its answer is never stored and never a census figure.
//!   * [`methodology`] — the provenance constants and rung-4 field list, as data.
//!   * [`validate`] — the pre-flight checker: judge a draft document before it
//!     is minted, using `crates/checks` unmodified.
//!   * [`subscribe`] — the only endpoint that stores something a person typed,
//!     and the only one that writes a row about anybody. Read its module doc
//!     before changing it: most of the care is in the parts that look like
//!     they are missing.
//!
//! `pub mod` (rather than plain `mod`) because `main.rs` needs to name these
//! handlers when building the router, e.g. `routes::agents::list`.

pub mod agents;
pub mod deltas;
pub mod findings;
pub mod methodology;
pub mod rates;
pub mod runs;
pub mod search;
pub mod sellers;
pub mod spot_check;
pub mod subscribe;
pub mod tail;
pub mod validate;
