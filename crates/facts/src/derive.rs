//! Measurement → Fact. Each function states exactly one claim and attaches
//! the evidence that backs it. Phrasing rules live here and nowhere else.

use chrono::{DateTime, Utc};
use serde_json::json;

use crate::model::*;

pub fn registered_since(r: &Registration) -> Fact {
    Fact {
        kind: "registered_since",
        value: json!({ "chain": r.chain, "registered_at": r.registered_at }),
        observed_at: r.registered_at,
        evidence: vec![EvidenceRef::Tx { chain: r.chain.clone(), tx_hash: r.tx_hash.clone() }],
    }
}

pub fn endpoint_liveness(s: &ProbeStats) -> Fact {
    Fact {
        kind: "endpoint_liveness",
        // Raw counts only. 100/120 alive is the fact; whether that's "good"
        // is the consumer's threshold, not ours.
        value: json!({ "probes": s.probes, "alive": s.alive, "window_from": s.from, "window_to": s.to }),
        observed_at: s.to,
        evidence: vec![EvidenceRef::ProbeWindow { from: s.from, to: s.to, probes: s.probes }],
    }
}

/// Only exists when at least one probe saw HTTP 402 — an endpoint that asks
/// for payment is alive AND payable, the x402 signal a naive health check
/// would misread as an error.
pub fn payable_endpoint(s: &ProbeStats) -> Option<Fact> {
    if s.payment_required == 0 {
        return None;
    }
    Some(Fact {
        kind: "payable_endpoint",
        value: json!({ "payment_required_responses": s.payment_required, "window_from": s.from, "window_to": s.to }),
        observed_at: s.to,
        evidence: vec![EvidenceRef::ProbeWindow { from: s.from, to: s.to, probes: s.probes }],
    })
}

pub fn metadata_status(s: &SnapshotStats, now: DateTime<Utc>) -> Fact {
    /// A card that hasn't resolved for this long is "rotted", not merely flaky.
    const ROT_AFTER_DAYS: i64 = 7;

    let (status, evidence) = match (s.last_ok_at, s.last_ok_snapshot_id) {
        (Some(ok_at), Some(id)) if (now - ok_at).num_days() >= ROT_AFTER_DAYS => {
            ("rotted", vec![EvidenceRef::Snapshot { snapshot_id: id }])
        }
        (Some(_), Some(id)) => ("resolving", vec![EvidenceRef::Snapshot { snapshot_id: id }]),
        _ => ("never_resolved", vec![]),
    };
    Fact {
        kind: "metadata_status",
        value: json!({
            "status": status,
            "last_resolved_at": s.last_ok_at,
            "last_checked_at": s.last_attempt_at,
            "snapshots_archived": s.total,
        }),
        observed_at: s.last_attempt_at.unwrap_or(now),
        evidence,
    }
}

pub fn attestations(s: &AttestationStats, chain: &str, now: DateTime<Utc>) -> Fact {
    Fact {
        kind: "attestations",
        // Phrased as counts: "N attestations recorded, M mutual". Mutuality is
        // published, not used to discount — consumers decide what it means.
        value: json!({ "total": s.total, "mutual": s.mutual }),
        observed_at: now,
        evidence: vec![EvidenceRef::Registry { chain: chain.to_string() }],
    }
}

pub fn validations(s: &ValidationStats, chain: &str, now: DateTime<Utc>) -> Fact {
    let status = if !s.registry_available {
        "registry_unavailable"
    } else if s.passed + s.failed == 0 {
        "absent"
    } else {
        "present"
    };
    Fact {
        kind: "validation_proofs",
        value: json!({ "status": status, "passed": s.passed, "failed": s.failed }),
        observed_at: now,
        evidence: vec![EvidenceRef::Registry { chain: chain.to_string() }],
    }
}
