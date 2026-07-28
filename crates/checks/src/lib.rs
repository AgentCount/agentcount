//! # checks — the conformance ladder, as pure functions.
//!
//! Seven questions, each answered `pass` / `fail` / `skipped` / `error`, each
//! carrying the evidence a reader can re-check by hand. There is no eighth
//! function that combines them into a number, and there never will be: the
//! absence of an aggregate is the product.
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
mod rung7_independent;
mod version;

pub use ladder::run_ladder;
pub use model::{CheckResult, CheckStatus};
pub use rung1_registered::{RegisteredInput, registered};
pub use rung2_resolvable::{ResolvableInput, resolvable};
pub use rung3_parseable::{ParseableInput, parseable};
pub use rung4_conformant::{ConformantInput, conformant};
pub use rung5_bound::{BoundInput, bound};
pub use rung7_independent::{IndependentInput, independent};
pub use version::{CHECKER_VERSION, SCHEMA_VERSION, SPEC_COMMIT};
