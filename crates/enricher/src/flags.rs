//! Flag production — evidence-backed coordination signals, one flag per
//! (agent, signal). Forensics is launch CONTENT and an optional flag producer,
//! not a penalty woven through the data model: no suspicion score, no cluster
//! multiplier. Every flag carries the concrete evidence a reader can check.
//!
//! Two signals, both derivable purely from what the Identity Registry gives us:
//! * **shared_operator** — the same wallet (NFT owner) controls several agents.
//! * **synchronized_registration** — a BURST of registrations in a tight
//!   window. Burst, not chain: naive consecutive-gap linking would merge a
//!   whole busy afternoon into one mega-cluster (any steady stream <120s apart
//!   chains transitively). We require ≥MIN_BURST_SIZE within a bounded span.
//!
//! A third signal — reciprocal feedback — lived here while feedback was modelled
//! as agent→agent. The deployed ERC-8004 Reputation Registry emits feedback as
//! client-ADDRESS→agent, so agent↔agent reciprocity no longer maps directly; a
//! replacement built on the owner-address↔agent mapping is future work.
//!
//! Rust concept spotlight: **pure core, async shell.** `detect_flags` is a
//! plain function over plain data — the heuristics that end up in the research
//! report are unit-tested without a database. `detect` is the thin async
//! wrapper that loads the inputs.

use std::collections::HashMap;

use serde_json::json;

/// Agents are identified by (chain, id) — the same numeric id exists on many
/// chains, so the chain is part of the identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentKey {
    pub chain: String,
    pub agent_id: i64,
}

/// The per-agent inputs, loaded from the database (address is address_norm —
/// lowercase, so string grouping can't fragment on case).
#[derive(Debug, Clone)]
pub struct AgentNode {
    pub key: AgentKey,
    pub address: String,
    pub registered_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlagKind {
    SharedOperator,
    SynchronizedRegistration,
}

impl FlagKind {
    /// Stable string stored in flags.kind and shown in the API/UI.
    pub fn label(&self) -> &'static str {
        match self {
            FlagKind::SharedOperator => "shared_operator",
            FlagKind::SynchronizedRegistration => "synchronized_registration",
        }
    }
}

/// One flag for one agent, evidence attached.
#[derive(Debug, Clone)]
pub struct AgentFlag {
    pub key: AgentKey,
    pub kind: FlagKind,
    pub evidence: serde_json::Value,
}

/// Registrations closer than this are "the same moment".
const BURST_GAP_SECS: i64 = 120;
/// A chain of close registrations only counts as a burst at this size — two
/// people registering in the same two minutes is coincidence, five is a script.
const MIN_BURST_SIZE: usize = 5;
/// A burst longer than this is split — prevents a busy day from becoming one flag.
const MAX_BURST_SPAN_SECS: i64 = 3_600;

/// Detect all flags across the agent set. Pure — inputs in, flags out.
pub fn detect_flags(nodes: &[AgentNode]) -> Vec<AgentFlag> {
    let mut out = Vec::new();

    // ── shared_operator ──────────────────────────────────────────────────────
    let mut by_address: HashMap<&str, Vec<&AgentNode>> = HashMap::new();
    for n in nodes {
        by_address.entry(n.address.as_str()).or_default().push(n);
    }
    for (address, group) in &by_address {
        if group.len() < 2 {
            continue;
        }
        for member in group {
            let peers: Vec<_> = group
                .iter()
                .filter(|p| p.key != member.key)
                .map(|p| json!({ "chain": p.key.chain, "agent_id": p.key.agent_id }))
                .collect();
            out.push(AgentFlag {
                key: member.key.clone(),
                kind: FlagKind::SharedOperator,
                evidence: json!({ "address": address, "peers": peers }),
            });
        }
    }

    // ── synchronized_registration (bursts, not chains) ───────────────────────
    let mut by_time: Vec<&AgentNode> = nodes.iter().collect();
    by_time.sort_by_key(|n| n.registered_at);
    let mut burst: Vec<&AgentNode> = Vec::new();
    // A closure that shares "flush" logic between the loop and the tail. It
    // captures nothing mutably (all state comes in as &mut params), so the
    // binding itself needn't be `mut`.
    let flush = |burst: &mut Vec<&AgentNode>, out: &mut Vec<AgentFlag>| {
        if burst.len() >= MIN_BURST_SIZE {
            let from = burst.first().unwrap().registered_at;
            let to = burst.last().unwrap().registered_at;
            for member in burst.iter() {
                let peers: Vec<_> = burst
                    .iter()
                    .filter(|p| p.key != member.key)
                    .map(|p| json!({ "chain": p.key.chain, "agent_id": p.key.agent_id }))
                    .collect();
                out.push(AgentFlag {
                    key: member.key.clone(),
                    kind: FlagKind::SynchronizedRegistration,
                    evidence: json!({ "window_from": from, "window_to": to, "count": burst.len(), "peers": peers }),
                });
            }
        }
        burst.clear();
    };
    for n in by_time {
        let breaks_gap = burst
            .last()
            .is_some_and(|prev| (n.registered_at - prev.registered_at).num_seconds() > BURST_GAP_SECS);
        let breaks_span = burst
            .first()
            .is_some_and(|first| (n.registered_at - first.registered_at).num_seconds() > MAX_BURST_SPAN_SECS);
        if breaks_gap || breaks_span {
            flush(&mut burst, &mut out);
        }
        burst.push(n);
    }
    flush(&mut burst, &mut out);

    out
}

/// Load inputs and run detection. Async shell around the pure core.
pub async fn detect(db: &crate::store::Db) -> anyhow::Result<Vec<AgentFlag>> {
    let nodes = db.load_agent_nodes().await?;
    Ok(detect_flags(&nodes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration, Utc};

    fn node(id: i64, address: &str, t0: DateTime<Utc>, offset_secs: i64) -> AgentNode {
        AgentNode {
            key: AgentKey { chain: "base".into(), agent_id: id },
            address: address.into(),
            registered_at: t0 + Duration::seconds(offset_secs),
        }
    }
    fn t0() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    #[test]
    fn shared_operator_flags_carry_the_address_and_peers() {
        let nodes = vec![
            node(1, "0xaaa", t0(), 0),
            node(2, "0xaaa", t0(), 500_000),
            node(3, "0xbbb", t0(), 0),
        ];
        let flags = detect_flags(&nodes);
        let shared: Vec<_> = flags.iter().filter(|f| f.kind == FlagKind::SharedOperator).collect();
        assert_eq!(shared.len(), 2, "both co-operated agents flagged, the loner not");
        let f = shared.iter().find(|f| f.key.agent_id == 1).unwrap();
        assert_eq!(f.evidence["address"], "0xaaa");
        assert_eq!(f.evidence["peers"][0]["agent_id"], 2);
    }

    /// The regression the old code had: a steady drip of registrations
    /// crossing hours must not chain into one giant cluster. The burst rule
    /// bounds every emitted window's SPAN, so long streams split into bounded
    /// windows and every flag names ITS window in the evidence.
    #[test]
    fn steady_traffic_splits_into_bounded_windows() {
        let many_hours: Vec<AgentNode> =
            (0..40).map(|i| node(100 + i, &format!("0y{i}"), t0(), i * 110)).collect();
        let flags = detect_flags(&many_hours);
        let sync: Vec<_> = flags.iter().filter(|f| f.kind == FlagKind::SynchronizedRegistration).collect();
        assert!(!sync.is_empty());
        for f in &sync {
            let from: DateTime<Utc> = serde_json::from_value(f.evidence["window_from"].clone()).unwrap();
            let to: DateTime<Utc> = serde_json::from_value(f.evidence["window_to"].clone()).unwrap();
            assert!(
                (to - from).num_seconds() <= MAX_BURST_SPAN_SECS,
                "burst window must be span-bounded, got {}s",
                (to - from).num_seconds()
            );
        }
    }

    #[test]
    fn small_coincidences_are_not_flagged() {
        // Four agents in two minutes: under MIN_BURST_SIZE → no flag.
        let nodes: Vec<AgentNode> =
            (0..4).map(|i| node(i, &format!("0x{i}"), t0(), i * 20)).collect();
        let flags = detect_flags(&nodes);
        assert!(flags.iter().all(|f| f.kind != FlagKind::SynchronizedRegistration));
    }
}
