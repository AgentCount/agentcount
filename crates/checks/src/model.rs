//! The vocabulary of a conformance result.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// The seven outcomes a rung can have, and nothing else.
///
/// There is deliberately no `Unknown` and no `Partial`: a rung either answered,
/// answered negatively, was not reached, broke on our side, was declined by the
/// origin, or — the two rung-specific additions below — found nothing to judge
/// because the agent made no claim for it to check. Anything fuzzier would be a
/// judgment in disguise.
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
    /// **Rungs 2 and 6**, added 2026-08-06. The origin is demonstrably there
    /// and declined to serve us. Neither the agent's failure nor ours.
    ///
    /// Three shapes of decline, one claim:
    ///
    /// * **"Come back later"** — HTTP 429 and 503, the two statuses RFC 9110
    ///   §10.2.3 / RFC 6585 §4 define as carrying `Retry-After`. Both name a
    ///   condition about *this request at this moment*, not about the document.
    /// * **A challenge** — HTTP 401, 402 and 407, the three statuses that
    ///   answer with a way in rather than an absence (`WWW-Authenticate`, a
    ///   payment challenge, `Proxy-Authenticate`). Something is there and it
    ///   wants credentials or money first.
    /// * **We were not given permission to ask** — `robots.txt` disallowed us,
    ///   or we could not establish permission from it at all. We honour that by
    ///   not sending the request, which means we never learned anything about
    ///   the document — see `rung2_resolvable`'s module doc for the full ruling
    ///   and `METHODOLOGY.md` §6.
    ///
    /// **Why it is not `fail`.** A 429 we caused by our own request rate,
    /// recorded as `fail`, publishes an infrastructure problem of ours as the
    /// agent's failure — which is exactly what happened: the 2026-08 census
    /// booked 19,983 BSC agents as having "stopped resolving", of which 19,962
    /// were 429s and 19,658 came from one host. Nothing about those documents
    /// changed.
    ///
    /// **Why it is not `error`.** `Error` means this checker malfunctioned, and
    /// none of these did. A robots.txt we honoured is a decision we made and
    /// can name; a 429 is an answer we received. Calling either a malfunction
    /// makes the error rate a measure of one host's mood — mainnet's read
    /// 22.1% when 6,133 agents on a single host had an unreadable robots.txt.
    ///
    /// It is not `pass` either, at any rung: we did not get the document, and
    /// (rung 6 excepted, see below) we did not establish that anything is live.
    /// Everything above it on the ladder is `skipped`, exactly as a `fail`
    /// would have skipped it — no agent's downstream rung moves because of this
    /// status.
    Refused,
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
            CheckStatus::Refused => "refused",
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
