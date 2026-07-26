//! Typed Rust bindings for the ERC-8004 registry contracts on Base.
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
//! The event signatures below are the REAL ones from the deployed ERC-8004 v1
//! registries on Base (Identity `Registered`, Reputation `NewFeedback`). The
//! canonical parameter types AND the `indexed` flags must match exactly, or
//! `log_decode` silently finds nothing (topic0 is keccak of the type list;
//! indexed-ness decides which fields live in topics vs data).

use alloy::sol;

sol! {
    // ── Identity Registry (0x8004A169…) ──────────────────────────────────────
    // Agents are ERC-721 NFTs. `Registered` fires on mint. `agentURI` is the
    // token URI pointing at the agent's metadata document (NOT a bare domain);
    // `owner` is the address that holds the NFT.
    #[derive(Debug)]
    event Registered(uint256 indexed agentId, string agentURI, address indexed owner);

    // ── Reputation Registry (0x8004BAa1…) ────────────────────────────────────
    // Feedback is left by an arbitrary CLIENT ADDRESS about an agent, carrying a
    // signed value with its own decimal scale plus free-form tags. All 11
    // parameters must be declared (in order, with correct indexed flags) so the
    // topic hash and topic/data split line up; we only read the first five.
    #[derive(Debug)]
    event NewFeedback(
        uint256 indexed agentId,
        address indexed clientAddress,
        uint64 feedbackIndex,
        int128 value,
        uint8 valueDecimals,
        string indexed indexedTag1,
        string tag1,
        string tag2,
        string endpoint,
        string feedbackURI,
        bytes32 feedbackHash
    );
}

/// A single decoded event, normalised into one enum the store can handle
/// uniformly regardless of which registry it came from.
///
/// Rust concept spotlight: **enums as tagged unions.** Each variant carries its
/// own differently-shaped data, and `match` forces the store to handle every
/// variant — add a new event later and the compiler lists every place to update.
#[derive(Debug)]
pub enum RegistryEvent {
    /// An agent NFT was minted. `agent_uri` is its metadata pointer; `owner` is
    /// the holding address (lower-cased).
    Registered {
        agent_id: u64,
        agent_uri: String,
        owner: String,
    },
    /// A client left feedback about an agent. `client_address` is the rater
    /// (lower-cased); `value`/`value_decimals` are the signed score and its
    /// scale, stored verbatim (we don't interpret the number yet).
    Feedback {
        to_agent_id: u64,
        client_address: String,
        feedback_index: i64,
        value: String,
        value_decimals: i16,
    },
}

/// The single place an on-chain address becomes a stored string. Lowercase by
/// contract with the schema (agents.address is documented lowercase, and
/// agents.address_norm enforces it as a backstop). alloy's `Display` renders
/// EIP-55 checksummed (mixed-case) hex — writing that would fragment every
/// string-equality join on address.
pub fn addr_lower(a: &alloy::primitives::Address) -> String {
    format!("{a:?}").to_lowercase() // Debug prints full 0x-prefixed hex
}

/// A decoded event bundled with the on-chain provenance the store needs to write
/// it: which block/tx it came from, when, and the raw payload for the audit log.
pub struct IndexedLog {
    pub chain: String,
    pub contract: String,
    pub event_name: &'static str,
    pub block: i64,
    pub tx_hash: String,
    pub block_hash: String,
    pub log_index: i32,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// A JSON snapshot of the decoded fields, stored in `raw_events.payload`.
    pub payload: serde_json::Value,
    pub event: RegistryEvent,
}

/// Turn one raw RPC log into an [`IndexedLog`]. The timestamp and block hash
/// come from the block HEADER the caller fetched — eth_getLogs responses do
/// not reliably carry timestamps, and falling back to "now" would poison the
/// longitudinal record with ingestion-time dates.
///
/// `Option` is Rust's null-free "maybe a value": callers `filter_map` over a
/// batch of logs and the `None`s simply drop out.
pub fn index_log(
    chain: &str,
    log: &alloy::rpc::types::Log,
    timestamp: chrono::DateTime<chrono::Utc>,
    block_hash: &str,
) -> Option<IndexedLog> {
    // These are `Option`s on the RPC log (a pending log has no block yet). The
    // `?` operator short-circuits to `None` if any is missing — we only index
    // fully-confirmed logs.
    let block = log.block_number? as i64;
    let tx_hash = log.transaction_hash?.to_string().to_lowercase();
    let log_index = log.log_index? as i32;
    let contract = addr_lower(&log.address());

    let (event_name, payload, event) = decode(log)?;

    Some(IndexedLog {
        chain: chain.to_string(),
        contract,
        event_name,
        block,
        tx_hash,
        block_hash: block_hash.to_string(),
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
    if let Ok(ev) = log.log_decode::<Registered>() {
        let d = ev.inner.data;
        let agent_id = d.agentId.to::<u64>();
        let agent_uri = d.agentURI;
        let owner = addr_lower(&d.owner);
        let payload = serde_json::json!({
            "agent_id": agent_id, "agent_uri": agent_uri.clone(), "owner": owner.clone(),
        });
        return Some((
            "Registered",
            payload,
            RegistryEvent::Registered { agent_id, agent_uri, owner },
        ));
    }

    if let Ok(ev) = log.log_decode::<NewFeedback>() {
        let d = ev.inner.data;
        let to_agent_id = d.agentId.to::<u64>();
        let client_address = addr_lower(&d.clientAddress);
        let feedback_index = d.feedbackIndex as i64;
        // `value` is int128; `.to_string()` gives the exact decimal regardless
        // of whether alloy backs it with a native or wide integer type.
        let value = d.value.to_string();
        let value_decimals = d.valueDecimals as i16;
        let payload = serde_json::json!({
            "to_agent_id": to_agent_id, "client_address": client_address.clone(),
            "feedback_index": feedback_index, "value": value.clone(),
            "value_decimals": value_decimals,
        });
        return Some((
            "NewFeedback",
            payload,
            RegistryEvent::Feedback {
                to_agent_id,
                client_address,
                feedback_index,
                value,
                value_decimals,
            },
        ));
    }

    None // unrecognised topic → not one of ours
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    /// Addresses are STORED lowercase (schema contract, 0001:23). alloy's
    /// Display renders EIP-55 checksummed (mixed-case) hex — writing that
    /// fragments every string-equality join on address. One helper, one rule.
    #[test]
    fn addresses_are_normalised_to_lowercase() {
        let a = address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045");
        assert_eq!(addr_lower(&a), "0xd8da6bf26964af9d7eed9e03e53415d37aa96045");
    }
}
