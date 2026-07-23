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
//! ## The formula, at a glance
//!
//! ```text
//! raw   = w_pay·payment + w_live·liveness + w_age·age + w_rep·reputation
//! final = raw · (1 − sybil_penalty)
//! ```
//!
//! The four positive sub-scores each live in `[0, 1]` and are blended by the
//! weights. The Sybil penalty is applied *multiplicatively* — that is the
//! mechanism that "discounts manufactured reputation": membership in a
//! coordinated cluster scales the whole score down no matter how good the
//! positive signals look.
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
// free to reshuffle the private modules later without breaking anyone. The `api`
// crate needs `FeedbackEdge` and `ClusterInfo` to build an `AgentView`, so those
// are part of the public surface too.
pub use model::{AgentView, ClusterInfo, FeedbackEdge, ScoreWeights, TrustScore};

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
/// Returns `Err` if the weights are invalid or any sub-score escapes `[0, 1]`
/// (which would indicate a bug in one of the sub-score functions).
pub fn score_with_weights(
    agent: &AgentView,
    weights: &ScoreWeights,
) -> Result<TrustScore, ScoringError> {
    // The `?` operator is Rust's error-propagation shortcut: if `validate`
    // returns `Err(e)`, `?` immediately returns that same `Err` from *this*
    // function. Clean, exception-like flow without exceptions.
    weights.validate()?;

    // Each sub-score is a pure function living in its own submodule. Keeping them
    // separate means each can be understood, tuned, and tested in isolation.
    let payment = subscores::payment::payment_score(agent);
    let liveness = subscores::liveness::liveness_score(agent);
    let age = subscores::age::age_score(agent);
    let reputation = subscores::reputation::reputation_score(agent);
    let sybil_penalty = subscores::sybil::sybil_penalty(agent);

    // Defensive range checks. Every sub-score *should* already clamp to [0, 1];
    // verifying it here turns a subtle numeric bug into a loud, named error
    // instead of a quietly wrong score served to the public. Cheap insurance.
    check_range("payment", payment)?;
    check_range("liveness", liveness)?;
    check_range("age", age)?;
    check_range("reputation", reputation)?;
    check_range("sybil_penalty", sybil_penalty)?;

    // Blend the positives, then apply the penalty as a multiplier.
    let raw = weights.payment * payment
        + weights.liveness * liveness
        + weights.age * age
        + weights.reputation * reputation;
    let final_score = raw * (1.0 - sybil_penalty);

    Ok(TrustScore {
        payment,
        liveness,
        age,
        reputation,
        sybil_penalty,
        final_score,
    })
}

/// Assert that a computed value is a real number inside `[0, 1]`.
///
/// Note that `(0.0..=1.0).contains(&value)` is already `false` for `NaN`
/// (every comparison with `NaN` is false), so this catches NaN too — a nice
/// property of leaning on the range's `contains` rather than hand-written `<`/`>`.
fn check_range(name: &'static str, value: f64) -> Result<(), ScoringError> {
    if (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(ScoringError::SubScoreOutOfRange { name, value })
    }
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
        // The published baseline must itself be a legal weight set.
        assert!(ScoreWeights::default().validate().is_ok());
    }

    #[test]
    fn weights_that_dont_sum_to_one_are_rejected() {
        let bad = ScoreWeights {
            payment: 0.5,
            liveness: 0.5,
            age: 0.5,
            reputation: 0.5, // sums to 2.0
        };
        assert!(matches!(
            bad.validate(),
            Err(ScoringError::WeightsDoNotSumToOne { .. })
        ));
    }

    #[test]
    fn a_healthy_agent_scores_in_range() {
        let agent = AgentView::sample();
        let s = score(&agent).expect("healthy agent should score cleanly");
        // Every component, and the final, must be a valid probability-like value.
        for v in [s.payment, s.liveness, s.age, s.reputation, s.sybil_penalty, s.final_score] {
            assert!((0.0..=1.0).contains(&v), "value {v} out of range");
        }
        // A lone, organic agent gets no Sybil penalty.
        assert_eq!(s.sybil_penalty, 0.0);
    }

    #[test]
    fn membership_in_a_tight_cluster_crushes_the_final_score() {
        // Take a strong agent and drop it into a big, coordinated cluster. The
        // multiplicative penalty should annihilate almost all of its score — the
        // headline behaviour of the whole methodology.
        let mut agent = AgentView::sample();
        agent.cluster.cluster_size = 1_000;
        agent.cluster.suspicion = 1.0;

        let s = score(&agent).unwrap();
        assert!(
            s.final_score < 0.01,
            "expected near-zero final score, got {}",
            s.final_score
        );
    }
}
