//! # facts — measurements with their proof attached, as pure functions
//!
//! The product publishes FACTS (evidence-backed, individually checkable
//! claims), never judgments. This crate is the one place a raw measurement
//! becomes a published claim, so its rules are strict:
//!
//! * every `Fact` carries `evidence` — the receipts a reader can check;
//! * values are raw counts and dates, never normalized scores;
//! * no I/O and no clock reads ("now" is a parameter), so the same inputs
//!   always yield the same fact — the methodology is reproducible.
//!
//! The `api` crate assembles the input structs from SQL and calls these
//! functions; nothing else decides what a published claim says.
//!
//! ## Rust concepts this crate is here to teach
//!
//! * **Modules & re-exports** — `mod derive; mod model;` split the crate;
//!   `pub use` re-exports the public surface so callers write `facts::Fact`.
//! * **Purity as an architectural rule** — like the old scoring crate, but the
//!   discipline now guards *phrasing* instead of math: with no clock and no
//!   I/O in here, "what we claim" is a deterministic function of "what we saw".

mod derive;
mod display;
mod model;

pub use derive::{
    attestations, endpoint_liveness, metadata_status, payable_endpoint, registered_since,
    validations,
};
pub use display::{FactDisplay, FlagDisplay, PublishedFact, describe, describe_flag};
pub use model::{
    AttestationStats, EvidenceRef, Fact, ProbeStats, Registration, SnapshotStats, ValidationStats,
};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration, Utc};

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn registration_fact_carries_the_tx_as_evidence() {
        let f = registered_since(&Registration {
            chain: "base".into(),
            registered_at: t(0),
            tx_hash: "0xabc".into(),
        });
        assert_eq!(f.kind, "registered_since");
        assert_eq!(f.value["chain"], "base");
        // The claim must be checkable: the registration tx is the proof.
        assert!(matches!(&f.evidence[0], EvidenceRef::Tx { tx_hash, .. } if tx_hash == "0xabc"));
    }

    #[test]
    fn liveness_fact_publishes_raw_counts_not_a_score() {
        let f = endpoint_liveness(&ProbeStats {
            from: t(0),
            to: t(86_400 * 30),
            probes: 120,
            alive: 100,
            payment_required: 10,
        });
        assert_eq!(f.value["probes"], 120);
        assert_eq!(f.value["alive"], 100);
        // No normalized rate, no percentage — consumers apply their own thresholds.
        assert!(f.value.get("rate").is_none() && f.value.get("score").is_none());
    }

    #[test]
    fn payable_endpoint_only_exists_when_a_402_was_observed() {
        let none = ProbeStats {
            from: t(0),
            to: t(60),
            probes: 5,
            alive: 5,
            payment_required: 0,
        };
        assert!(payable_endpoint(&none).is_none());
        let some = ProbeStats {
            from: t(0),
            to: t(60),
            probes: 5,
            alive: 5,
            payment_required: 3,
        };
        assert_eq!(payable_endpoint(&some).unwrap().kind, "payable_endpoint");
    }

    #[test]
    fn metadata_rot_is_a_dated_claim_with_the_last_good_snapshot() {
        let f = metadata_status(
            &SnapshotStats {
                total: 10,
                last_ok_at: Some(t(0)),
                last_ok_snapshot_id: Some(42),
                last_attempt_at: Some(t(0) + Duration::days(9)),
            },
            t(0) + Duration::days(10),
        );
        assert_eq!(f.value["status"], "rotted");
        assert_eq!(f.value["last_resolved_at"], serde_json::json!(t(0)));
        // Evidence points at the archived snapshot — the content may be gone
        // from the origin, but we kept what it said.
        assert!(matches!(
            f.evidence[0],
            EvidenceRef::Snapshot { snapshot_id: 42 }
        ));
    }

    #[test]
    fn never_resolved_and_resolving_statuses() {
        let never = metadata_status(
            &SnapshotStats {
                total: 3,
                last_ok_at: None,
                last_ok_snapshot_id: None,
                last_attempt_at: Some(t(5)),
            },
            t(10),
        );
        assert_eq!(never.value["status"], "never_resolved");

        let fresh = metadata_status(
            &SnapshotStats {
                total: 3,
                last_ok_at: Some(t(5)),
                last_ok_snapshot_id: Some(7),
                last_attempt_at: Some(t(5)),
            },
            t(10),
        );
        assert_eq!(fresh.value["status"], "resolving");
    }

    #[test]
    fn validations_fact_distinguishes_absent_registry_from_zero_proofs() {
        let unavailable = validations(
            &ValidationStats {
                registry_available: false,
                passed: 0,
                failed: 0,
            },
            "base",
            t(0),
        );
        assert_eq!(unavailable.value["status"], "registry_unavailable");

        let none = validations(
            &ValidationStats {
                registry_available: true,
                passed: 0,
                failed: 0,
            },
            "base",
            t(0),
        );
        assert_eq!(none.value["status"], "absent");
    }
}
