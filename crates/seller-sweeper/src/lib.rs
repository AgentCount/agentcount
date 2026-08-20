//! # seller-sweeper — the Seller Census's I/O half.
//!
//! `crates/sellers` decides things; this crate fetches and writes. The split
//! is the same one `crates/probe` and `crates/checks` keep, for the same
//! reason: a rule that can only be exercised against a live catalog is a
//! rule nobody re-checks, so no rule lives here.
//!
//! What does live here:
//!
//! * [`fetcher`] — polite HTTP against catalogs, with robots.txt honoured
//!   through `crates/probe`'s one implementation (METHODOLOGY §10.3) and
//!   every response hashed, because a hashed snapshot is what replaces a
//!   pinned block for a population that lives off-chain.
//! * [`store`] — the writes, against migration 0026's tables.
//!
//! The binaries:
//!
//! * **`seller-crawl`** — one pass over the catalogs: snapshot, assemble,
//!   write the population and rung 1.

pub mod fetcher;
pub mod store;

/// The product token this census identifies itself with when it talks to a
/// catalog — distinct from the agent prober's, so a catalog operator reading
/// their logs can tell the two instruments apart and can rate-limit or
/// disallow either one independently.
pub const PRODUCT_TOKEN: &str = "agentcount-sellers";

/// The `User-Agent` string every request from this crate carries. It names
/// the methodology so that anyone who sees this traffic can read what it is
/// for before deciding what to do about it.
pub const USER_AGENT: &str =
    "agentcount-sellers/0.1 (+https://agentcount.ai/methodology; census@agentcount.ai)";
