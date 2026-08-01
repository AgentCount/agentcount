//! The vocabulary of a conformance result.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// The six outcomes a rung can have, and nothing else.
///
/// There is deliberately no `Unknown` and no `Partial`: a rung either answered,
/// answered negatively, was not reached, broke on our side, or — the two
/// rung-specific additions below — found nothing to judge because the agent
/// made no claim for it to check. Anything fuzzier would be a judgment in
/// disguise.
///
/// A seventh case exists and is deliberately NOT a variant here: a rung that
/// was never asked has no row at all. "We did not ask" and "we asked and got
/// nothing" are different claims, and the absence of a row is what keeps them
/// different — see `ladder`'s doc comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    /// A lower rung did not pass, so this question could not be asked.
    Skipped,
    /// OUR failure — a timeout in our prober, an RPC error. Never the agent's.
    Error,
    /// **Rung 5 (`bound`) only**, added by the rung-5 status fix (2026-07-29).
    /// Since P0 FIX 3 made `registrations` a SHOULD rather than a MUST, a
    /// document can pass rung 4 while carrying no `registrations` array at
    /// all — and rung 5's entire question ("does the document's own
    /// registration entry match the on-chain record we fetched it from") has
    /// nothing to check in that case. None of the other four statuses is
    /// honest for it: `pass` would claim a verification that never happened,
    /// `fail` would punish a merely-recommended field, `skipped` would
    /// falsely imply an earlier rung failed, and `error` would falsely imply
    /// this checker malfunctioned. `Unclaimed` names the case precisely: the
    /// agent made no binding claim for this rung to verify. See
    /// `rung5_bound`'s module doc for the full reasoning and
    /// `METHODOLOGY.md` §2 for the published definition.
    Unclaimed,
    /// **Rung 6 (`live`) only**, added with rung 6 itself (2026-08-01).
    ///
    /// Rung 6 asks whether anything answers at the endpoints the agent
    /// declared. Only `http`/`https` endpoints can be probed, and 11.0% of
    /// declared "endpoints" across the census are not network endpoints at all
    /// — they are CAIP-10 chain addresses, email addresses, empty strings, or
    /// the `endpoint` field is simply absent. An agent whose every declared
    /// entry is one of those has nothing for this rung to reach.
    ///
    /// This is exactly [`CheckStatus::Unclaimed`]'s reasoning applied one rung
    /// over, and the same four rejections hold: `pass` would claim a liveness
    /// nobody demonstrated, `fail` would punish an agent for declining to
    /// publish a URL when the spec does not require one, `skipped` would
    /// falsely imply rung 4 stopped it, and `error` would falsely imply this
    /// checker malfunctioned.
    ///
    /// Distinct from `Unclaimed` on purpose rather than reusing it: an agent
    /// that published a CAIP-10 address made a claim, it is simply not one a
    /// prober can dial. Collapsing "declared something unprobeable" into
    /// "declared nothing" would erase that. See `rung6_live`'s module doc and
    /// `METHODOLOGY.md` §2.
    Unprobeable,
}

impl CheckStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckStatus::Pass => "pass",
            CheckStatus::Fail => "fail",
            CheckStatus::Skipped => "skipped",
            CheckStatus::Error => "error",
            CheckStatus::Unclaimed => "unclaimed",
            CheckStatus::Unprobeable => "unprobeable",
        }
    }
}

/// One rung's answer about one agent, with the proof attached.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub rung: u8,
    pub name: &'static str,
    pub status: CheckStatus,
    /// Structured per rung — never prose. What a reader re-checks by hand.
    pub evidence: serde_json::Value,
    pub checked_at: DateTime<Utc>,
}
