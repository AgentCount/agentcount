//! # probe — polite, guarded fetching of an agent's declared registration document.
//!
//! This crate owns HTTP and nothing else. `Prober::fetch` takes one agent's
//! raw `tokenURI()` string and returns a [`FetchOutcome`]: what we observed,
//! as plain data. It never decides pass/fail/error — no rung logic lives
//! here — that judgment belongs entirely to `crates/checks`, which stays
//! pure by never importing this crate's networking.
//!
//! Roughly 23,744 of the ~60,000 agents in the population need a real
//! network request (`https://`, `http://`, or `ipfs://` via a gateway); the
//! rest resolve to `Empty`, `Inline` (decoded `data:` payload, five fallback
//! decode paths, P0 FIX 7), or `Unsupported`/`UnsupportedCompression`
//! without ever touching the network. See [`resolve::resolve`] for the
//! classification and [`fetch::Prober`] for the guarded fetch path.

mod fetch;
mod netguard;
mod resolve;
mod robots;

pub use fetch::{
    DEFAULT_GLOBAL_CONCURRENCY, FetchOutcome, MAX_BODY_BYTES, MAX_REDIRECTS, PER_HOST_CAP, Prober,
};
pub use resolve::{DataUriDecode, Target, resolve};

/// Re-exported at crate root for anything (tests, future crates) that wants
/// the exact product token this prober identifies itself with, without
/// reaching into `fetch`.
pub(crate) use fetch::PRODUCT_TOKEN;
