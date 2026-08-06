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
///
/// 6 (2026-07-30): minter capture. `agent_snapshots` gains `minter`,
/// `registration_tx_hash` and `registration_block` (migration 0013), and
/// rung 1's `tx_hash` evidence field — which has always existed and has
/// always been `null` — is now populated from the registration transaction.
///
/// **No rung's rule changes and no agent's status moves.** Rung 1's verdict
/// depends only on whether `owner` is the zero address; `tx_hash` has only
/// ever been evidence. The bump exists so a reader can tell from the row
/// alone whether a null `tx_hash` means "this run did not capture it"
/// (schema ≤ 5) or "we looked and the chain had nothing" (schema 6) — the
/// same distinction between *did not ask* and *asked and got nothing* that
/// the six statuses keep everywhere else.
/// 7 (2026-08-01): **rung 6 (`live`) ships.** The service track stops being
/// absent from every result and starts producing rows — the largest single
/// change to what a run contains since the ladder was written.
///
/// - `check_results.status` gains a sixth value, `unprobeable`, produced only
///   by rung 6 for an agent whose every declared endpoint is something no
///   prober can dial: a CAIP-10 chain address, an email address, an empty
///   string, or an absent `endpoint` field (migration 0015 widens the `CHECK`
///   constraint accordingly).
/// - A new table, `endpoint_probes`, archives one row per (run, URL) rather
///   than per agent, because one URL is declared by many agents.
/// - Rung 6's evidence contract is defined for the first time:
///   `endpoints_declared` / `_probeable` / `_probed` / `_live` /
///   `_payment_gated` / `_answered_not_live` / `_our_error`, plus a per-entry
///   `endpoints[]` array carrying each declared string, its kind and its own
///   outcome.
///
/// **No other rung's rule changes and no existing agent's status moves.** A
/// row written under schema ≤ 6 has no rung-6 sibling, and that absence
/// continues to mean what it always meant: not asked. The bump is how a reader
/// tells, from the row alone, whether a missing rung 6 means "this run did not
/// implement it" (schema ≤ 6) or "this run implemented it and this agent was
/// not probed" (schema 7) — the same *did not ask* versus *asked and got
/// nothing* distinction the statuses keep everywhere else.
///
/// 8 (2026-08-06): **`refused`** — a third answer for "the origin is there and
/// declined us", produced by rungs 2 and 6, and **the first bump that moves
/// existing agents' statuses**.
///
/// - `check_results.status` gains a seventh value (migration 0020 widens the
///   `CHECK` constraint). On rung 2 it takes HTTP 429/503/401/402/407 out of
///   `fail`, and `robots_disallowed`/`robots_unavailable` out of `error`. On
///   rung 6 it takes 429/503/401/407 and the same two robots outcomes out of
///   `fail`/`error`; 402 is unchanged there and still `pass`.
/// - Rung 2's evidence gains `retry_after` (seconds), present only when a 429
///   or 503 carried the header, and its `reason` for a decline is `declined`
///   (or the pre-existing `payment_required` for a 402). Rung 6's evidence
///   gains `endpoints_refused`.
/// - No rung's PASS rule changes, no agent gains or loses a `pass`, and
///   skip-propagation is unaffected — `refused` is not `pass`, so it stops a
///   dependent rung exactly as the `fail` or `error` it replaced did.
///
/// **Runs swept under schema ≤ 7 are re-judged, not left inconsistent**, by
/// `sweeper`'s `backfill-refused` binary, which re-reads the archived evidence
/// and restamps each run it touches to this version. A run stamped 8 has been
/// judged by this vocabulary whether it was swept under it or re-judged into
/// it; the published archives of the 2026-07 and 2026-08 censuses were written
/// before the backfill and carry the old words — see `DATA.md` and the
/// 2026-08-06 changelog entry for the exact mapping.
pub const SCHEMA_VERSION: i32 = 8;

/// The checks crate's own version — the semantics of the rungs.
pub const CHECKER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The ERC8004SPEC.md commit these checks were written against. Must match
/// `spec/SOURCE.md`; update both together or a result claims to have been
/// judged against a spec it was not.
pub const SPEC_COMMIT: &str = "68fc6765761a10fb26f0692df21c8a6f9d12b1be";
