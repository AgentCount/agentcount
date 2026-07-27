//! The vocabulary of a conformance result.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// The four outcomes a rung can have, and nothing else.
///
/// There is deliberately no `Unknown` and no `Partial`: a rung either answered,
/// answered negatively, was not reached, or broke on our side. Anything fuzzier
/// would be a judgment in disguise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    /// A lower rung did not pass, so this question could not be asked.
    Skipped,
    /// OUR failure — a timeout in our prober, an RPC error. Never the agent's.
    Error,
}

impl CheckStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckStatus::Pass => "pass",
            CheckStatus::Fail => "fail",
            CheckStatus::Skipped => "skipped",
            CheckStatus::Error => "error",
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
