//! The route handlers, grouped by what they serve.
//!
//! Splitting handlers into files by resource keeps `main.rs`'s router readable
//! and each file focused:
//!   * [`agents`] — JSON endpoints about individual agents.
//!   * [`stats`]  — the aggregate "how much reputation is fake" numbers.
//!   * [`pages`]  — the server-rendered HTML explorer.
//!
//! `pub mod` (rather than plain `mod`) because `main.rs` needs to name these
//! handlers when building the router, e.g. `routes::agents::list`.

pub mod agents;
pub mod pages;
pub mod stats;
