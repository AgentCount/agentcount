//! The vocabulary: what a Fact is and what counts as evidence.
//!
//! Rust concept spotlight: **enums with data + serde tagging.** `EvidenceRef`
//! is a tagged union; `#[serde(tag = "type")]` makes each variant serialize as
//! `{"type": "tx", ...}` so API consumers can dispatch on the tag.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// A pointer to the proof behind a claim — something a reader can go check.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvidenceRef {
    /// An on-chain transaction.
    Tx { chain: String, tx_hash: String },
    /// An archived metadata snapshot row — the archive of what a domain said
    /// at a point in time, even after the origin rots.
    Snapshot { snapshot_id: i64 },
    /// A window of probe history rows.
    ProbeWindow {
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        probes: i64,
    },
    /// The on-chain registry itself (for counts derived wholly from indexed events).
    Registry { chain: String },
}

/// One published, checkable claim. `value` is raw measurements (counts,
/// dates, statuses) — never a normalized score; consumers threshold for
/// themselves. `kind` is a stable string the API and frontend key on.
#[derive(Debug, Clone, Serialize)]
pub struct Fact {
    pub kind: &'static str,
    pub value: serde_json::Value,
    pub observed_at: DateTime<Utc>,
    pub evidence: Vec<EvidenceRef>,
}

// ── Inputs, assembled from SQL by the api crate ──────────────────────────────

pub struct Registration {
    pub chain: String,
    pub registered_at: DateTime<Utc>,
    pub tx_hash: String,
}

pub struct ProbeStats {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub probes: i64,
    /// Probes where the endpoint was reachable and answering (2xx or 402).
    pub alive: i64,
    /// Probes that returned HTTP 402 — the x402 "alive and payable" signal.
    pub payment_required: i64,
}

pub struct SnapshotStats {
    pub total: i64,
    pub last_ok_at: Option<DateTime<Utc>>,
    pub last_ok_snapshot_id: Option<i64>,
    pub last_attempt_at: Option<DateTime<Utc>>,
}

pub struct AttestationStats {
    /// On-chain feedback rows pointing at this agent. In the deployed ERC-8004
    /// model each is left by a client address, so this is a raw count of
    /// feedback received — no agent↔agent pairing is asserted.
    pub total: i64,
}

pub struct ValidationStats {
    /// False when this chain has no Validation Registry (per-chain variance) —
    /// "no proofs" and "no registry to hold proofs" are different claims.
    pub registry_available: bool,
    pub passed: i64,
    pub failed: i64,
}
