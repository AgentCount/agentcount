//! The five signals that make up a trust score, one per submodule.
//!
//! Each submodule exposes a single pure function taking `&AgentView` and
//! returning an `f64` in `[0, 1]`. Splitting them apart like this is the point:
//! you can read, tune, and test each signal in isolation, and the top-level
//! [`crate::score_with_weights`] just blends the results.
//!
//! Rust concept spotlight: **module visibility.** These submodules are declared
//! `pub(crate)` — visible *within* this crate but not to outside callers. The
//! outside world uses [`crate::score`]; the individual signals are an internal
//! implementation detail we keep private so we can change them freely.

pub(crate) mod payment;
pub(crate) mod reputation;
pub(crate) mod liveness;
pub(crate) mod age;
pub(crate) mod sybil;

// A shared helper every submodule can use. `pub(super)` = visible to the parent
// module (`subscores`) and its children. Clamping guarantees a stray formula can
// never emit, say, 1.3 and quietly corrupt the final score.
/// Force `x` into the closed interval `[0, 1]`.
pub(super) fn clamp01(x: f64) -> f64 {
    // `f64::clamp` is in the standard library. Written out so the intent is
    // obvious to a reader new to Rust.
    x.clamp(0.0, 1.0)
}
