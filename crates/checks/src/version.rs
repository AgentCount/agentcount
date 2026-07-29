//! Provenance constants stamped onto every run.
//!
//! `CHECKER_COMMIT` is injected by the sweeper's build script rather than
//! declared here, because this crate does not know how it was built.

/// Bump when the shape of `check_results` or the evidence contract changes.
///
/// 2 (P0 FIX 3): rung 4's evidence dropped `fields_found`/`fields_missing`
/// in favor of `must_violations[]`/`should_gaps[]`/`may_gaps[]`, and gained
/// `services_status`. The `evidence` column is `jsonb`, so old rows are
/// still readable — this bump exists so a reader can tell, from the row
/// alone, which evidence contract produced it, per Section 5 of
/// `METHODOLOGY.md`.
pub const SCHEMA_VERSION: i32 = 2;

/// The checks crate's own version — the semantics of the rungs.
pub const CHECKER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The ERC8004SPEC.md commit these checks were written against. Must match
/// `spec/SOURCE.md`; update both together or a result claims to have been
/// judged against a spec it was not.
pub const SPEC_COMMIT: &str = "68fc6765761a10fb26f0692df21c8a6f9d12b1be";
