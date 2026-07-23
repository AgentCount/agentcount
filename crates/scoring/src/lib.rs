//! # scoring — the trust methodology, as pure functions
//!
//! This crate answers one question: *given everything we know about an agent,
//! how much should we trust its on-chain reputation?* It is the intellectual
//! core of Ledgerscope and the thing the launch research post is really about.
//!
//! ## Why this crate has no I/O
//!
//! There is no database code, no HTTP, no `async` anywhere in here. Everything
//! it needs arrives as a plain [`AgentView`] value, and everything it produces
//! leaves as a plain [`TrustScore`] value. That is a deliberate design choice
//! with big payoffs:
//!
//! * **Testability** — a pure function is just `input -> output`. You can write
//!   a hundred unit tests without spinning up Postgres or mocking a network.
//! * **Determinism** — same input, same score, forever. Great for a *published*
//!   methodology people are meant to audit and reproduce.
//! * **Portability** — the `api` crate can call [`score`] on demand, or a batch
//!   job can call it on a schedule. The scoring logic doesn't know or care.
//!
//! ## Rust concepts this crate is here to teach
//!
//! * **Modules** — `mod model;` and `mod subscores;` below split one crate into
//!   a tree of files. `pub use` then re-exports the important names so callers
//!   write `scoring::TrustScore`, not `scoring::model::TrustScore`.
//! * **Ownership & borrowing** — [`score`] takes `&AgentView` (a *borrow*): it
//!   reads the agent without taking ownership, so the caller keeps its data.
//! * **Typed errors** — [`ScoringError`] is a `thiserror` enum, the library-
//!   idiomatic way to report *why* something failed so callers can `match` on it.

// ── Module declarations ──────────────────────────────────────────────────────
// `mod foo;` tells the compiler "there is a sibling file foo.rs (or foo/mod.rs);
// compile it as a submodule of this crate." Without these lines the files exist
// on disk but are invisible to the compiler.
mod model;
mod subscores;

// ── Re-exports: the crate's public front door ────────────────────────────────
// Callers of this crate should not need to know our internal file layout. By
// re-exporting the key types here, `scoring::TrustScore` just works, and we stay
// free to reshuffle the private modules later without breaking anyone.
pub use model::{AgentView, ScoreWeights, TrustScore};

use thiserror::Error;

/// Everything that can go wrong while scoring.
///
/// Because this is a library, we hand callers a precise enum they can `match`
/// on, rather than a stringly-typed blob. `#[derive(Error)]` (from `thiserror`)
/// auto-generates the boilerplate that makes this a real `std::error::Error`,
/// and each `#[error("...")]` string is the human-readable message.
#[derive(Debug, Error)]
pub enum ScoringError {
    /// The weights supplied by the caller don't sum to 1.0 (within tolerance),
    /// so the final score wouldn't stay inside the intended `[0, 1]` range.
    #[error("score weights must sum to 1.0, but summed to {actual}")]
    WeightsDoNotSumToOne { actual: f64 },

    /// A sub-score computed to something outside `[0, 1]` — a bug in the math
    /// we'd rather catch loudly than silently serve a nonsense number.
    #[error("sub-score '{name}' was {value}, outside the valid [0, 1] range")]
    SubScoreOutOfRange { name: &'static str, value: f64 },
}

/// Compute an agent's trust score using the DEFAULT published weights.
///
/// This is the convenience entry point most callers want. It simply forwards to
/// [`score_with_weights`] using [`ScoreWeights::default`].
///
/// The `&AgentView` argument is a *shared borrow*: we promise only to read the
/// agent, never to mutate or consume it, so the caller keeps ownership and can
/// keep using its value afterwards.
pub fn score(agent: &AgentView) -> Result<TrustScore, ScoringError> {
    score_with_weights(agent, &ScoreWeights::default())
}

/// Compute an agent's trust score using caller-supplied weights.
///
/// The formula, in one line:
/// ```text
/// raw   = w_pay·payment + w_live·liveness + w_age·age + w_rep·reputation
/// final = raw · (1 − sybil_penalty)
/// ```
/// The multiplicative `sybil_penalty` is the mechanism that "discounts
/// manufactured reputation": no matter how good the four positive sub-scores
/// look, membership in a suspicious cluster drags the final number down.
///
/// Returns `Err` if the weights are invalid or any sub-score escapes `[0, 1]`.
pub fn score_with_weights(
    agent: &AgentView,
    weights: &ScoreWeights,
) -> Result<TrustScore, ScoringError> {
    // The `?` operator below is Rust's error-propagation shortcut: if the called
    // function returns `Err(e)`, `?` immediately returns `Err(e)` from *this*
    // function too. It's how you get clean, exception-like flow without
    // exceptions. Uncomment once `validate` exists:
    //
    //     weights.validate()?;

    // Each sub-score is a pure function living in its own submodule. They return
    // a value in [0, 1]. We deliberately keep them separate so each can be
    // understood, tuned, and tested in isolation.
    //
    //     let payment    = subscores::payment::payment_score(agent);
    //     let liveness   = subscores::liveness::liveness_score(agent);
    //     let age        = subscores::age::age_score(agent);
    //     let reputation = subscores::reputation::reputation_score(agent);
    //     let sybil      = subscores::sybil::sybil_penalty(agent);
    //
    //     let raw = weights.payment    * payment
    //             + weights.liveness   * liveness
    //             + weights.age        * age
    //             + weights.reputation * reputation;
    //
    //     let final_score = raw * (1.0 - sybil);
    //
    //     Ok(TrustScore { payment, liveness, age, reputation,
    //                     sybil_penalty: sybil, final_score })

    let _ = (agent, weights); // silence "unused" warnings until you fill this in
    todo!("combine the five sub-scores into a TrustScore (see the sketch above)")
}

// ── Tests live in the same file as the code they test ────────────────────────
// This `#[cfg(test)]` module is compiled ONLY when running `cargo test`, never
// in a release build. Colocating tests with code is the normal Rust convention
// for unit tests.
#[cfg(test)]
mod tests {
    // `use super::*;` pulls in everything from the parent module (this file), so
    // the tests can see `score`, `TrustScore`, etc.
    use super::*;

    #[test]
    fn default_weights_are_valid() {
        // A first, easy invariant to lock down once `validate` exists:
        // the default published weights must themselves be a legal weight set.
        //
        //     assert!(ScoreWeights::default().validate().is_ok());
        let _ = ScoreWeights::default();
        todo!("assert the default weights sum to 1.0");
    }

    // Ideas for further tests as you build the math out:
    //   * a fully-clustered agent (sybil_penalty = 1.0) always scores 0.0
    //   * more counterparty diversity never lowers the payment sub-score
    //   * a dead endpoint caps the liveness sub-score at 0.0
}
