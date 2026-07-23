//! Typed Rust bindings for the ERC-8004 registry contracts.
//!
//! This is where alloy's headline feature — the `sol!` macro — earns its keep.
//! You hand it Solidity (a contract interface, or just the event signatures you
//! care about) and at COMPILE TIME it generates matching Rust structs plus the
//! code to decode raw log data into them. No hand-written ABI parsing, no
//! stringly-typed field access: a mistyped event field becomes a compiler error.
//!
//! Rust concept spotlight: **procedural macros.** `#[derive(Serialize)]` you've
//! already met generates code from a struct. `sol! { ... }` is a *procedural*
//! macro that generates code from an entirely different language embedded in
//! your Rust source. Same core idea — code that writes code at compile time —
//! taken to its logical extreme. This is why the RPC/ABI story feels so tidy.

// The real thing looks like this. `sol!` expands each `event` into a Rust struct
// (e.g. `AgentRegistered { agent_id, agent_domain, agent_address }`) with a
// decoder you can apply to a log. The Solidity below is illustrative — replace
// the signatures with the exact ones from the ERC-8004 registry ABIs, which are
// the source of truth for field names, types, and `indexed`-ness.
//
//     use alloy::sol;
//
//     sol! {
//         // ── Identity Registry ───────────────────────────────────────────
//         // Emitted when a new agent registers. `indexed` fields become log
//         // topics (cheap to filter on); non-indexed fields live in log data.
//         event AgentRegistered(
//             uint256 indexed agentId,
//             string  agentDomain,
//             address indexed agentAddress
//         );
//
//         // ── Reputation Registry ─────────────────────────────────────────
//         // One agent authorises/records feedback about another.
//         event FeedbackGiven(
//             uint256 indexed fromAgentId,
//             uint256 indexed toAgentId,
//             uint8   score
//         );
//
//         // ── Validation Registry ─────────────────────────────────────────
//         // A validator attests to the outcome of some work.
//         event ValidationRecorded(
//             uint256 indexed validatorId,
//             uint256 indexed subjectId,
//             bool    passed
//         );
//     }
//
// After that macro runs, you can write strongly-typed code like:
//
//     if let Ok(ev) = AgentRegistered::decode_log(&raw_log) {
//         // ev.agentId, ev.agentDomain, ev.agentAddress — all typed.
//     }

/// A single decoded event, normalised into one enum the ingest loop can handle
/// uniformly regardless of which registry it came from.
///
/// Rust concept spotlight: **enums as tagged unions.** Unlike a C enum (just
/// named integers), a Rust `enum` variant can *carry data*, and each variant can
/// carry a different shape. `match` then forces you to handle every variant —
/// add a new event type later and the compiler lists every place that needs
/// updating. This is "make illegal states unrepresentable" in action.
#[derive(Debug)]
pub enum RegistryEvent {
    /// A new agent appeared in the Identity Registry.
    AgentRegistered {
        agent_id: u64,
        domain: String,
        // In the real code this is an `alloy::primitives::Address`.
        agent_address: String,
    },
    /// Feedback recorded in the Reputation Registry.
    FeedbackGiven {
        from_agent_id: u64,
        to_agent_id: u64,
        score: u8,
    },
    /// A validation outcome recorded in the Validation Registry.
    ValidationRecorded {
        validator_id: u64,
        subject_id: u64,
        passed: bool,
    },
}

/// Try to decode one raw log into a [`RegistryEvent`].
///
/// Returns `Option` because not every log we receive is one we understand — an
/// unrecognised event simply yields `None` and the caller skips it. `Option` is
/// Rust's null-free way to say "maybe there's a value here".
///
/// The `RawLog` parameter is a placeholder for alloy's log type
/// (`alloy::rpc::types::Log`); swap it in when you wire alloy up.
pub fn decode_log(_raw: &RawLog) -> Option<RegistryEvent> {
    // Real shape, once the `sol!` block above exists:
    //
    //     if let Ok(ev) = AgentRegistered::decode_log(raw) {
    //         return Some(RegistryEvent::AgentRegistered {
    //             agent_id: ev.agentId.to::<u64>(),
    //             domain: ev.agentDomain,
    //             agent_address: ev.agentAddress.to_string(),
    //         });
    //     }
    //     if let Ok(ev) = FeedbackGiven::decode_log(raw) { /* ... */ }
    //     if let Ok(ev) = ValidationRecorded::decode_log(raw) { /* ... */ }
    //     None // unrecognised topic → not one of ours

    todo!("match the log's topic0 against each sol!-generated event and decode it")
}

/// Placeholder for alloy's raw log type. Delete when alloy is wired in.
pub struct RawLog;
