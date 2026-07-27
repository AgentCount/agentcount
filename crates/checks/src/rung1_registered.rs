//! Rung 1 — `registered`: the agent id exists in the Identity Registry and an
//! ERC-721 token for it is held by a non-zero address.
//!
//! This rung is nearly always a pass, and that is fine: it establishes the
//! denominator. Every base rate published later is "of the N agents that pass
//! rung 1", so this is the rung that defines the population.

use chrono::{DateTime, Utc};
use serde_json::json;

use crate::model::{CheckResult, CheckStatus};

/// The chain reads rung 1 judges. Assembled by the sweeper from a
/// `chain::AgentSnapshot`; this crate never learns how they were obtained.
#[derive(Debug, Clone)]
pub struct RegisteredInput {
    pub chain_id: u64,
    /// Registry contract address, lowercase hex.
    pub registry: String,
    /// ERC-721 token id as a decimal STRING — uint256 does not fit i64 and a
    /// JSON number would lose precision above 2^53.
    pub token_id: String,
    /// Owner from `ownerOf()`, lowercase hex.
    pub owner: String,
    pub block_number: u64,
    /// The registration transaction, when we know it. `None` is recorded as
    /// null — never as an empty string, which would look like a real value.
    pub tx_hash: Option<String>,
}

const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";

pub fn registered(input: &RegisteredInput, now: DateTime<Utc>) -> CheckResult {
    let mut evidence = json!({
        "chain_id": input.chain_id,
        "registry": input.registry,
        "token_id": input.token_id,
        "owner": input.owner,
        "block_number": input.block_number,
        "tx_hash": input.tx_hash,
    });

    let status = if input.owner.eq_ignore_ascii_case(ZERO_ADDRESS) {
        evidence["reason"] = json!("owner_is_zero_address");
        CheckStatus::Fail
    } else {
        CheckStatus::Pass
    };

    CheckResult { rung: 1, name: "registered", status, evidence, checked_at: now }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CheckStatus;
    use chrono::{DateTime, Utc};

    fn t() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    fn input() -> RegisteredInput {
        RegisteredInput {
            chain_id: 8453,
            registry: "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432".into(),
            token_id: "42".into(),
            owner: "0xabc0000000000000000000000000000000000001".into(),
            block_number: 41_817_815,
            tx_hash: Some("0xdead".into()),
        }
    }

    #[test]
    fn a_held_token_passes_and_carries_the_full_evidence_set() {
        let r = registered(&input(), t());
        assert_eq!(r.rung, 1);
        assert_eq!(r.name, "registered");
        assert_eq!(r.status, CheckStatus::Pass);
        // Every field the product spec lists as rung-1 evidence.
        assert_eq!(r.evidence["chain_id"], 8453);
        assert_eq!(r.evidence["registry"], "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432");
        assert_eq!(r.evidence["token_id"], "42");
        assert_eq!(r.evidence["owner"], "0xabc0000000000000000000000000000000000001");
        assert_eq!(r.evidence["block_number"], 41_817_815u64);
        assert_eq!(r.evidence["tx_hash"], "0xdead");
    }

    #[test]
    fn the_zero_address_owner_is_a_fail_not_a_pass() {
        // A burned or non-existent token: ownerOf would revert, but a registry
        // that returns the zero address must not read as "registered".
        let mut i = input();
        i.owner = "0x0000000000000000000000000000000000000000".into();
        let r = registered(&i, t());
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["reason"], "owner_is_zero_address");
    }

    #[test]
    fn evidence_is_recorded_even_when_the_rung_fails() {
        let mut i = input();
        i.owner = "0x0000000000000000000000000000000000000000".into();
        let r = registered(&i, t());
        // A failing rung must still say what it saw — that is the whole product.
        assert_eq!(r.evidence["chain_id"], 8453);
        assert_eq!(r.evidence["token_id"], "42");
    }

    #[test]
    fn a_missing_tx_hash_is_null_not_an_invented_value() {
        let mut i = input();
        i.tx_hash = None;
        let r = registered(&i, t());
        assert!(r.evidence["tx_hash"].is_null());
    }
}
