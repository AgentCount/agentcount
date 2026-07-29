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
///
/// 3 (P0 FIX 4/5, 2026-07-29): three changes land together, per the work
/// order's requirement that fixes 4 and 5 ship in one run —
/// - `check_results.status` gains a fifth value, `unclaimed`, produced only
///   by rung 5 (`bound`) when a document carries no `registrations` claim to
///   check (migration 0011 widens the `CHECK` constraint accordingly).
/// - Rung 7's `name` changes from `independent` to `attested`, and its
///   evidence drops `authors_equal_to_owner`/`self_feedback_ratio` (both
///   depended on an owner comparison this rung no longer makes — see
///   `rung7_attested`'s module doc).
/// - Rung 7 is ungated: it now runs for every agent that passes rung 1,
///   not only those that also pass rungs 2 through 5.
///
/// 4 (P0 FIX 7, 2026-07-29): rung 2's (`resolvable`) evidence gains
/// `data_uri_variant`/`data_uri_algorithm` for a `"data"`-scheme result —
/// which of the five `data:` decode fallback paths produced the bytes, and
/// the compression algorithm when one was involved. No field is removed and
/// no existing field changes meaning; old rows remain readable without
/// either key present.
///
/// 5 (P0 FIX 8, 2026-07-29): rung 2's evidence gains `gateway_attempts` for
/// an `"ipfs"`-scheme result — every gateway tried, in order, with each
/// one's own status, now that `crates/probe` tries up to three gateways in
/// sequence instead of one. `via_gateway` is unchanged in meaning (still the
/// winner, when there was one). No field removed; old rows remain readable
/// without the new key.
pub const SCHEMA_VERSION: i32 = 5;

/// The checks crate's own version — the semantics of the rungs.
pub const CHECKER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The ERC8004SPEC.md commit these checks were written against. Must match
/// `spec/SOURCE.md`; update both together or a result claims to have been
/// judged against a spec it was not.
pub const SPEC_COMMIT: &str = "68fc6765761a10fb26f0692df21c8a6f9d12b1be";
