//! Fact → prose. The one place a published claim is put into words.
//!
//! `derive.rs` decides WHAT is true; this module decides HOW WE SAY IT. Both
//! are pure, so the sentence a reader sees is a deterministic function of the
//! measurement — and it is written exactly once, for every consumer. Before
//! this module existed the API served raw values while the HTML pages built
//! their own sentences, so the two could drift without anything failing.
//!
//! Rust concept spotlight: **borrowing for read-only work.** Every function
//! here takes `&Fact` and returns freshly-owned `String`s. The caller keeps
//! ownership of its fact; we only need to look at it.

use chrono::DateTime;
use serde::Serialize;

use crate::model::{EvidenceRef, Fact};

/// A fact rendered for a human. Every string is display-ready: consumers
/// concatenate nothing and interpret nothing.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FactDisplay {
    /// e.g. "Endpoint liveness"
    pub label: String,
    /// e.g. "answered 100 of 120 probes in the last 30 days"
    pub statement: String,
    /// e.g. "120 archived probes"; empty when the fact carries no evidence.
    pub evidence_summary: String,
}

/// How many days a probe-window fact covers, read from the fact itself.
///
/// The window length belongs to whoever queried the database, so restating a
/// literal "30" here would be a second definition of it — exactly the drift
/// this module exists to remove. `?` on `Option` makes the whole chain bail
/// out to `None` the moment anything is missing or unparseable.
fn window_days(v: &serde_json::Value) -> Option<i64> {
    let from = DateTime::parse_from_rfc3339(v["window_from"].as_str()?).ok()?;
    let to = DateTime::parse_from_rfc3339(v["window_to"].as_str()?).ok()?;
    Some((to - from).num_days())
}

/// Extract a numeric field with a `?` placeholder for missing or malformed data.
///
/// Evidence is data we wrote, but also data we might have written wrong: a
/// measurement we never took must not render as a confident number. When a
/// count is absent or stored as a string rather than a number, `?` shows the
/// fallback rather than printing `null` or a quoted string.
fn num(v: &serde_json::Value) -> String {
    match v.as_i64() {
        Some(n) => n.to_string(),
        None => "?".to_string(),
    }
}

/// Put one fact into words. Unknown kinds degrade to the raw JSON rather than
/// panicking: a future `derive.rs` addition should render *something* useful
/// even before it is taught a sentence here.
pub fn describe(f: &Fact) -> FactDisplay {
    let v = &f.value;
    let (label, statement) = match f.kind {
        "registered_since" => (
            "Registered".to_string(),
            format!(
                "since {} on {}",
                v["registered_at"].as_str().unwrap_or("?"),
                v["chain"].as_str().unwrap_or("?")
            ),
        ),
        "endpoint_liveness" => (
            "Endpoint liveness".to_string(),
            match window_days(v) {
                Some(days) => format!(
                    "answered {} of {} probes in the last {days} days",
                    num(&v["alive"]),
                    num(&v["probes"])
                ),
                None => format!(
                    "answered {} of {} probes",
                    num(&v["alive"]),
                    num(&v["probes"])
                ),
            },
        ),
        "payable_endpoint" => (
            "Payable endpoint".to_string(),
            format!(
                "returned HTTP 402 (payment required) on {} probes",
                num(&v["payment_required_responses"])
            ),
        ),
        "metadata_status" => (
            "Metadata".to_string(),
            format!(
                "{} ({} snapshots archived)",
                v["status"].as_str().unwrap_or("?"),
                num(&v["snapshots_archived"])
            ),
        ),
        "attestations" => (
            "Attestations".to_string(),
            format!("{} recorded on-chain", num(&v["total"])),
        ),
        "validation_proofs" => (
            "Validation proofs".to_string(),
            format!(
                "{} ({} passed, {} failed)",
                v["status"].as_str().unwrap_or("?"),
                num(&v["passed"]),
                num(&v["failed"])
            ),
        ),
        other => (other.to_string(), v.to_string()),
    };

    FactDisplay {
        label,
        statement,
        evidence_summary: f
            .evidence
            .iter()
            .map(|e| match e {
                EvidenceRef::Tx { chain, tx_hash } => format!("tx {tx_hash} ({chain})"),
                EvidenceRef::Snapshot { snapshot_id } => format!("snapshot #{snapshot_id}"),
                EvidenceRef::ProbeWindow { probes, .. } => format!("{probes} archived probes"),
                EvidenceRef::Registry { chain } => format!("{chain} registry events"),
            })
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// A flag rendered for a human. Flags have no `value`/`evidence` split — the
/// evidence JSON *is* the claim — so one statement covers it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FlagDisplay {
    /// e.g. "shared operator"
    pub label: String,
    /// e.g. "operated by the same wallet (0x…) as 4 other agent(s)"
    pub statement: String,
}

/// Put one flag into words.
///
/// `kind` is a `&str`, not an enum, on purpose: `flags.kind` is a TEXT column
/// and the `FlagKind` enum lives in the `enricher` BINARY, which a library
/// must not depend on. The stable-string contract already exists
/// (`FlagKind::label()`); this function is its reader, and an unrecognised
/// kind degrades to a readable label rather than an error.
pub fn describe_flag(kind: &str, evidence: &serde_json::Value) -> FlagDisplay {
    let peers = evidence["peers"].as_array().map(|p| p.len()).unwrap_or(0);
    let statement = match kind {
        "shared_operator" => format!(
            "operated by the same wallet ({}) as {peers} other agent(s)",
            evidence["address"].as_str().unwrap_or("?")
        ),
        "synchronized_registration" => format!(
            "registered in a burst of {} agents within one window",
            num(&evidence["count"])
        ),
        _ => String::new(),
    };
    FlagDisplay {
        label: kind.replace('_', " "),
        statement,
    }
}

/// A fact as it crosses the wire: the measurement, plus the words we publish
/// about it.
///
/// `Fact` itself stays a pure measurement type — this wrapper is the API's
/// shape, not the model's. `#[serde(flatten)]` splices the fact's own fields
/// into this struct's JSON object instead of nesting them under a `fact` key,
/// so existing consumers still find `kind`/`value` exactly where they were.
///
/// Shipping presentation strings in a data API is normally a smell. Here the
/// precise wording IS the product: "answered 100 of 120 probes" is a claim we
/// are accountable for, and re-deriving it downstream is how a frontend ends
/// up contradicting the API.
#[derive(Debug, Clone, Serialize)]
pub struct PublishedFact {
    #[serde(flatten)]
    pub fact: Fact,
    pub display: FactDisplay,
}

impl PublishedFact {
    pub fn new(fact: Fact) -> Self {
        let display = describe(&fact);
        Self { fact, display }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use chrono::{DateTime, Duration, Utc};

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn registration_reads_as_a_dated_claim() {
        let f = crate::registered_since(&Registration {
            chain: "base".into(),
            registered_at: t(0),
            tx_hash: "0xabc".into(),
        });
        let d = describe(&f);
        assert_eq!(d.label, "Registered");
        assert_eq!(d.statement, "since 2023-11-14T22:13:20Z on base");
        assert_eq!(d.evidence_summary, "tx 0xabc (base)");
    }

    #[test]
    fn liveness_states_raw_counts_and_the_window_it_measured() {
        let f = crate::endpoint_liveness(&ProbeStats {
            from: t(0),
            to: t(0) + Duration::days(30),
            probes: 120,
            alive: 100,
            payment_required: 10,
        });
        let d = describe(&f);
        assert_eq!(d.label, "Endpoint liveness");
        // The window length is READ FROM THE FACT, never hardcoded — the
        // window is chosen by the api crate and must not be restated here.
        assert_eq!(
            d.statement,
            "answered 100 of 120 probes in the last 30 days"
        );
        assert_eq!(d.evidence_summary, "120 archived probes");
    }

    #[test]
    fn payable_endpoint_names_the_402() {
        let f = crate::payable_endpoint(&ProbeStats {
            from: t(0),
            to: t(60),
            probes: 5,
            alive: 5,
            payment_required: 3,
        })
        .unwrap();
        let d = describe(&f);
        assert_eq!(d.label, "Payable endpoint");
        assert_eq!(
            d.statement,
            "returned HTTP 402 (payment required) on 3 probes"
        );
    }

    #[test]
    fn metadata_status_reports_status_and_archive_depth() {
        let f = crate::metadata_status(
            &SnapshotStats {
                total: 10,
                last_ok_at: Some(t(0)),
                last_ok_snapshot_id: Some(42),
                last_attempt_at: Some(t(0) + Duration::days(9)),
            },
            t(0) + Duration::days(10),
        );
        let d = describe(&f);
        assert_eq!(d.label, "Metadata");
        assert_eq!(d.statement, "rotted (10 snapshots archived)");
        assert_eq!(d.evidence_summary, "snapshot #42");
    }

    #[test]
    fn attestations_and_validations_read_as_counts() {
        let a = describe(&crate::attestations(
            &AttestationStats { total: 7 },
            "base",
            t(0),
        ));
        assert_eq!(a.label, "Attestations");
        assert_eq!(a.statement, "7 recorded on-chain");
        assert_eq!(a.evidence_summary, "base registry events");

        let v = describe(&crate::validations(
            &ValidationStats {
                registry_available: true,
                passed: 2,
                failed: 1,
            },
            "base",
            t(0),
        ));
        assert_eq!(v.label, "Validation proofs");
        assert_eq!(v.statement, "present (2 passed, 1 failed)");
    }

    #[test]
    fn an_unknown_kind_degrades_instead_of_panicking() {
        let f = Fact {
            kind: "some_future_fact",
            value: serde_json::json!({ "n": 1 }),
            observed_at: t(0),
            evidence: vec![],
        };
        let d = describe(&f);
        assert_eq!(d.label, "some_future_fact");
        assert_eq!(d.statement, r#"{"n":1}"#);
        assert_eq!(d.evidence_summary, "");
    }

    #[test]
    fn a_liveness_fact_without_a_parseable_window_drops_the_window_clause() {
        let f = Fact {
            kind: "endpoint_liveness",
            value: serde_json::json!({ "probes": 4, "alive": 2 }),
            observed_at: t(0),
            evidence: vec![],
        };
        assert_eq!(describe(&f).statement, "answered 2 of 4 probes");
    }

    #[test]
    fn shared_operator_names_the_wallet_and_the_peer_count() {
        let d = describe_flag(
            "shared_operator",
            &serde_json::json!({
                "address": "0xdead",
                "peers": [
                    { "chain": "base", "agent_id": 2 },
                    { "chain": "base", "agent_id": 3 }
                ]
            }),
        );
        assert_eq!(d.label, "shared operator");
        assert_eq!(
            d.statement,
            "operated by the same wallet (0xdead) as 2 other agent(s)"
        );
    }

    #[test]
    fn synchronized_registration_names_the_burst_size() {
        let d = describe_flag(
            "synchronized_registration",
            &serde_json::json!({
                "window_from": "2026-05-01T00:00:00Z",
                "window_to": "2026-05-01T00:10:00Z",
                "count": 6,
                "peers": [{ "chain": "base", "agent_id": 2 }]
            }),
        );
        assert_eq!(d.label, "synchronized registration");
        assert_eq!(
            d.statement,
            "registered in a burst of 6 agents within one window"
        );
    }

    #[test]
    fn flag_evidence_with_missing_keys_does_not_panic() {
        let d = describe_flag("shared_operator", &serde_json::json!({}));
        assert_eq!(d.label, "shared operator");
        assert_eq!(
            d.statement,
            "operated by the same wallet (?) as 0 other agent(s)"
        );
    }

    #[test]
    fn an_unknown_flag_kind_still_gets_a_readable_label() {
        let d = describe_flag("some_future_signal", &serde_json::json!({}));
        assert_eq!(d.label, "some future signal");
        assert_eq!(d.statement, "");
    }

    #[test]
    fn synchronized_registration_with_missing_count_renders_placeholder() {
        let d = describe_flag(
            "synchronized_registration",
            &serde_json::json!({
                "window_from": "2026-05-01T00:00:00Z",
                "window_to": "2026-05-01T00:10:00Z",
                "peers": [{ "chain": "base", "agent_id": 2 }]
            }),
        );
        assert_eq!(d.label, "synchronized registration");
        assert_eq!(
            d.statement,
            "registered in a burst of ? agents within one window"
        );
    }

    #[test]
    fn endpoint_liveness_with_missing_counts_renders_placeholders() {
        let f = Fact {
            kind: "endpoint_liveness",
            value: serde_json::json!({}),
            observed_at: t(0),
            evidence: vec![],
        };
        assert_eq!(describe(&f).statement, "answered ? of ? probes");
    }

    #[test]
    fn published_fact_serializes_flat_with_display_attached() {
        let f = crate::attestations(&AttestationStats { total: 7 }, "base", t(0));
        let json = serde_json::to_value(PublishedFact::new(f)).unwrap();

        // The raw measurement stays exactly where machine consumers expect it,
        // at the top level — `display` is purely additive.
        assert_eq!(json["kind"], "attestations");
        assert_eq!(json["value"]["total"], 7);
        assert!(json["observed_at"].is_string());
        assert!(json["evidence"].is_array());
        assert_eq!(json["display"]["label"], "Attestations");
        assert_eq!(json["display"]["statement"], "7 recorded on-chain");
        assert_eq!(json["display"]["evidence_summary"], "base registry events");
    }
}
