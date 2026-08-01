//! # checks — the conformance ladder, as pure functions.
//!
//! Seven questions, each answered `pass` / `fail` / `skipped` / `error` — plus
//! two rung-specific words for the agent that gave a rung nothing to judge:
//! `unclaimed` for rung 5 (`bound`), added 2026-07-29 (see
//! [`CheckStatus::Unclaimed`]), and `unprobeable` for rung 6 (`live`), added
//! 2026-08-01 (see [`CheckStatus::Unprobeable`]). Each carries the evidence a
//! reader can re-check by hand. There is no eighth function that combines them
//! into a number, and there never will be: the absence of an aggregate is the
//! product.
//!
//! Rungs 1 through 7 are three INDEPENDENT tracks, not one chain — see the
//! `ladder` module's doc comment for the full dependency graph. Document
//! (1→2→3→4→5), Service (6) and Reputation (7) each have their own internal
//! dependency; nothing outside a track can skip a rung inside another one.
//! Rung 6 (`live`) depends on rung 4 alone — it needs a document that conforms
//! enough to declare `services`, not rung 5's binding re-litigated. Rung 7
//! (`attested`, renamed from `independent` on 2026-07-29) depends on rung 1
//! alone.
//!
//! Purity is the load-bearing property. No I/O, no clock, no randomness —
//! so the same inputs always yield the same result, and a published finding
//! can be reproduced years later from the archived evidence alone.

mod ladder;
mod model;
mod rung1_registered;
mod rung2_resolvable;
mod rung3_parseable;
mod rung4_conformant;
mod rung5_bound;
mod rung6_live;
mod rung7_attested;
mod version;

pub use ladder::run_ladder;
pub use model::{CheckResult, CheckStatus};
pub use rung1_registered::{RegisteredInput, registered};
pub use rung2_resolvable::{ResolvableInput, resolvable};
pub use rung3_parseable::{ParseableInput, parseable};
pub use rung4_conformant::{
    ConformantInput, MAY_FIELDS, REGISTRATION_ENTRY_FIELDS, SHOULD_SPECIAL_FIELDS,
    SHOULD_TOP_LEVEL_FIELDS, conformant,
};
pub use rung5_bound::{BoundInput, bound};
pub use rung6_live::{
    EndpointKind, EndpointObservation, LiveInput, ServiceEndpoint, classify_endpoint, live,
};
pub use rung7_attested::{AttestedInput, attested};
pub use version::{CHECKER_VERSION, SCHEMA_VERSION, SPEC_COMMIT};
