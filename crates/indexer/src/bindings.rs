//! Typed Rust bindings for the ERC-8004 registry contracts.
//!
//! This is where alloy's headline feature — the `sol!` macro — earns its keep.
//! You hand it Solidity event signatures and at COMPILE TIME it generates
//! matching Rust structs plus the code to decode raw log data into them. No
//! hand-written ABI parsing, no stringly-typed field access.
//!
//! Rust concept spotlight: **procedural macros.** `#[derive(Serialize)]`
//! generates code from a struct; `sol! { ... }` generates code from Solidity
//! embedded in your Rust source. Same idea — code that writes code at compile
//! time — taken to its logical extreme.
//!
//! ⚠️ The event signatures below are illustrative. Replace them with the exact
//! signatures from the real ERC-8004 registry ABIs (field names, types, and
//! which fields are `indexed` must match, or decoding silently finds nothing).

use alloy::sol;

// `sol!` expands each `event` into a Rust struct implementing the `SolEvent`
// trait, which knows the event's topic hash and how to decode a log into typed
// fields. `#[derive(Debug)]` is passed through to the generated structs.
sol! {
    // ── Identity Registry ───────────────────────────────────────────────────
    // Emitted when a new agent registers. `indexed` fields become searchable log
    // topics; the rest live in the log's data section.
    #[derive(Debug)]
    event AgentRegistered(uint256 indexed agentId, string agentDomain, address indexed agentAddress);

    // ── Reputation Registry ─────────────────────────────────────────────────
    #[derive(Debug)]
    event FeedbackGiven(uint256 indexed fromAgentId, uint256 indexed toAgentId, uint8 score);

    // ── Validation Registry ─────────────────────────────────────────────────
    #[derive(Debug)]
    event ValidationRecorded(uint256 indexed validatorId, uint256 indexed subjectId, bool passed);
}

/// A single decoded event, normalised into one enum the store can handle
/// uniformly regardless of which registry it came from.
///
/// Rust concept spotlight: **enums as tagged unions.** Each variant carries its
/// own differently-shaped data, and `match` forces the store to handle every
/// variant — add a new event later and the compiler lists every place to update.
#[derive(Debug)]
pub enum RegistryEvent {
    AgentRegistered {
        agent_id: u64,
        domain: String,
        agent_address: String,
    },
    FeedbackGiven {
        from_agent_id: u64,
        to_agent_id: u64,
        score: u8,
    },
    ValidationRecorded {
        validator_id: u64,
        subject_id: u64,
        passed: bool,
    },
}

/// A decoded event bundled with the on-chain provenance the store needs to write
/// it: which block/tx it came from, when, and the raw payload for the audit log.
pub struct IndexedLog {
    pub chain: String,
    pub contract: String,
    pub event_name: &'static str,
    pub block: i64,
    pub tx_hash: String,
    pub log_index: i32,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// A JSON snapshot of the decoded fields, stored in `raw_events.payload`.
    pub payload: serde_json::Value,
    pub event: RegistryEvent,
}

/// Turn one raw RPC log into an [`IndexedLog`], or `None` if it isn't an event we
/// recognise or is missing the block/tx metadata we require.
///
/// `Option` is Rust's null-free "maybe a value": callers `filter_map` over a
/// batch of logs and the `None`s simply drop out.
pub fn index_log(chain: &str, log: &alloy::rpc::types::Log) -> Option<IndexedLog> {
    // These are `Option`s on the RPC log (a pending log has no block yet). The
    // `?` operator short-circuits to `None` if any is missing — we only index
    // fully-confirmed logs.
    let block = log.block_number? as i64;
    let tx_hash = log.transaction_hash?.to_string();
    let log_index = log.log_index? as i32;
    let contract = log.address().to_string();

    // Use the block timestamp if the RPC provided one; otherwise fall back to
    // ingestion time (a wall-clock read, which is fine in a binary).
    let timestamp = log
        .block_timestamp
        .and_then(|ts| chrono::DateTime::from_timestamp(ts as i64, 0))
        .unwrap_or_else(chrono::Utc::now);

    let (event_name, payload, event) = decode(log)?;

    Some(IndexedLog {
        chain: chain.to_string(),
        contract,
        event_name,
        block,
        tx_hash,
        log_index,
        timestamp,
        payload,
        event,
    })
}

/// Try each known event type in turn. `log.log_decode::<T>()` succeeds only if
/// the log's topic matches `T`, so trying them one after another is a clean way
/// to route a log to the right decoder.
fn decode(
    log: &alloy::rpc::types::Log,
) -> Option<(&'static str, serde_json::Value, RegistryEvent)> {
    if let Ok(ev) = log.log_decode::<AgentRegistered>() {
        let d = ev.inner.data;
        let agent_id = d.agentId.to::<u64>();
        let domain = d.agentDomain;
        let address = d.agentAddress.to_string();
        let payload = serde_json::json!({
            "agent_id": agent_id, "domain": domain, "address": address,
        });
        return Some((
            "AgentRegistered",
            payload,
            RegistryEvent::AgentRegistered {
                agent_id,
                domain,
                agent_address: address,
            },
        ));
    }

    if let Ok(ev) = log.log_decode::<FeedbackGiven>() {
        let d = ev.inner.data;
        let from_agent_id = d.fromAgentId.to::<u64>();
        let to_agent_id = d.toAgentId.to::<u64>();
        let score = d.score;
        let payload = serde_json::json!({
            "from_agent_id": from_agent_id, "to_agent_id": to_agent_id, "score": score,
        });
        return Some((
            "FeedbackGiven",
            payload,
            RegistryEvent::FeedbackGiven {
                from_agent_id,
                to_agent_id,
                score,
            },
        ));
    }

    if let Ok(ev) = log.log_decode::<ValidationRecorded>() {
        let d = ev.inner.data;
        let validator_id = d.validatorId.to::<u64>();
        let subject_id = d.subjectId.to::<u64>();
        let passed = d.passed;
        let payload = serde_json::json!({
            "validator_id": validator_id, "subject_id": subject_id, "passed": passed,
        });
        return Some((
            "ValidationRecorded",
            payload,
            RegistryEvent::ValidationRecorded {
                validator_id,
                subject_id,
                passed,
            },
        ));
    }

    None // unrecognised topic → not one of ours
}
