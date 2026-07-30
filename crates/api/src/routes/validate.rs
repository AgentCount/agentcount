//! The pre-flight checker: judge a registration document before it is minted.
//!
//! `POST /api/validate` takes a draft registration file and answers the rungs
//! that can be answered without an on-chain agent behind it.
//!
//! ## This adds no check logic
//!
//! Every verdict here comes from `crates/checks`, unmodified, through the same
//! functions the sweeper calls:
//!
//! ```text
//!   parseable(ParseableInput { body, .. })      → rung 3
//!   conformant(ConformantInput { document })    → rung 4
//!   bound(BoundInput { document, actual_* })    → rung 5
//!   run_ladder(vec![..])                        → the gating between them
//! ```
//!
//! That is deliberate and load-bearing. A separate validator — even a faithful
//! one — would be a second implementation free to drift, and the day it
//! disagreed with the census the census would be the thing people stopped
//! trusting. The answer this endpoint gives IS the answer the census would
//! give, because it is the same code.
//!
//! In particular the RUNG GATING is not reimplemented. `run_ladder` decides
//! that a document which fails rung 3 gets `skipped` at rungs 4 and 5, and it
//! decides it here exactly as it does during a sweep.
//!
//! ## What it cannot answer, and why absence is the honest answer
//!
//! Rungs 1 (`registered`), 2 (`resolvable`) and 7 (`attested`) are not
//! answerable before minting: there is no agent id yet, no published URI to
//! fetch, and no feedback that could exist. They are therefore **absent from
//! the response** rather than reported as passing, failing, or skipped.
//!
//! Absence already means "not checked" everywhere else in this product, and
//! the frontend renders it that way without knowing this endpoint exists. So a
//! pre-flight result and a real census result are read with the same
//! vocabulary, and neither can be mistaken for the other: a draft simply has
//! four rungs nobody has asked yet.
//!
//! Rung 5 (`bound`) is included ONLY when the caller supplies the identity it
//! intends to register under. Without it there is nothing to compare the
//! document's `registrations` entry against, and guessing an identity would
//! manufacture a verdict. No identity supplied → no rung 5 row → "not checked".

use axum::Json;
use axum::body::Bytes;
use axum::extract::Query;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use std::fmt::Write as _;

use crate::error::{ApiError, ApiResult};

/// A body cap, so a paste cannot be used to make this process do unbounded
/// work. Chosen to match `crates/probe`'s own archive cap: a document larger
/// than the sweeper would have kept is not a document this endpoint should
/// pretend it can judge.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// The identity a builder intends to register under. All three or none — a
/// partial identity cannot be compared against a `registrations` entry, and
/// filling the gaps with defaults would invent the very claim rung 5 exists to
/// verify.
#[derive(Debug, Deserialize)]
pub struct ValidateParams {
    pub agent_id: Option<u64>,
    pub chain_id: Option<u64>,
    pub registry: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ValidateRung {
    pub rung: u8,
    pub name: &'static str,
    pub status: String,
    pub evidence: serde_json::Value,
    pub checked_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct ValidateResponse {
    /// The same provenance a run carries, so a pre-flight answer can be tied
    /// to the exact checker and spec pin that produced it.
    pub checker_version: &'static str,
    pub schema_version: i32,
    pub spec_commit: &'static str,
    pub body_bytes: usize,
    pub body_sha256: String,
    /// Only the rungs that could be asked. A rung absent from this list was
    /// not checked — never assume a status for it.
    pub rungs: Vec<ValidateRung>,
    /// Why each absent rung is absent, in the checker's own terms. Presentation
    /// is the frontend's job; this is the reason, not the sentence.
    pub not_applicable: Vec<NotApplicable>,
}

#[derive(Debug, Serialize)]
pub struct NotApplicable {
    pub rung: u8,
    pub name: &'static str,
    pub reason: &'static str,
}

/// `POST /api/validate?agent_id=&chain_id=&registry=`
///
/// The body is the document itself, as bytes, whatever its content type — the
/// point of rung 3 is to find out whether those bytes parse, so they are
/// passed through untouched rather than being deserialised on the way in.
pub async fn post(
    Query(params): Query<ValidateParams>,
    body: Bytes,
) -> ApiResult<Json<ValidateResponse>> {
    if body.is_empty() {
        return Err(ApiError::BadRequest(
            "no document supplied — POST the registration file as the request body".into(),
        ));
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(ApiError::BadRequest(format!(
            "document is {} bytes; the limit is {MAX_BODY_BYTES}",
            body.len()
        )));
    }

    // All three or none. A partial identity is a caller mistake worth naming,
    // not something to silently half-apply.
    let intended = match (params.agent_id, params.chain_id, params.registry.as_deref()) {
        (Some(agent_id), Some(chain_id), Some(registry)) if !registry.trim().is_empty() => {
            Some((agent_id, chain_id, registry.trim().to_lowercase()))
        }
        (None, None, None) => None,
        _ => {
            return Err(ApiError::BadRequest(
                "supply all of agent_id, chain_id and registry to check rung 5 (bound), or none of them".into(),
            ));
        }
    };

    let now = Utc::now();
    // Hex by hand rather than pulling in the `hex` crate for one call.
    let body_sha256 = Sha256::digest(&body)
        .iter()
        .fold(String::with_capacity(64), |mut acc, b| {
            let _ = write!(acc, "{b:02x}");
            acc
        });

    // ── Rung 3, from the raw bytes ─────────────────────────────────────────
    let (rung3, document) = checks::parseable(
        &checks::ParseableInput {
            body: Some(body.to_vec()),
            // A paste has no meaningful transport content type, and rung 3
            // records it in evidence without ever gating on it.
            content_type: None,
            body_sha256: Some(body_sha256.clone()),
            // This endpoint never truncates: it rejects an over-large body
            // outright rather than judging a cut-off document, which would
            // blame the author for our limit.
            truncated: false,
        },
        now,
    );

    // ── Rung 4 ─────────────────────────────────────────────────────────────
    // Constructed even when the document did not parse, exactly as the sweeper
    // does (P0 FIX 6): `run_ladder` below is what turns it into `skipped`, and
    // deciding that here instead would be reimplementing the gating.
    let rung4 = checks::conformant(
        &checks::ConformantInput {
            document: document.clone().unwrap_or(serde_json::Value::Null),
        },
        checks::SPEC_COMMIT,
        now,
    );

    let mut rungs = vec![rung3, rung4];

    // ── Rung 5, only against a stated identity ─────────────────────────────
    if let Some((agent_id, chain_id, registry)) = &intended {
        rungs.push(checks::bound(
            &checks::BoundInput {
                document: document.unwrap_or(serde_json::Value::Null),
                actual_agent_id: *agent_id,
                actual_chain_id: *chain_id,
                actual_registry: registry.clone(),
            },
            now,
        ));
    }

    // The gating, from the checker. Not reimplemented here.
    let rungs = checks::run_ladder(rungs);

    let mut not_applicable = vec![
        NotApplicable {
            rung: 1,
            name: "registered",
            reason: "no on-chain agent id exists until the document is minted",
        },
        NotApplicable {
            rung: 2,
            name: "resolvable",
            reason: "there is no published URI to fetch for a draft",
        },
        NotApplicable {
            rung: 6,
            name: "live",
            reason: "not implemented for any agent",
        },
        NotApplicable {
            rung: 7,
            name: "attested",
            reason: "feedback cannot exist before the agent does",
        },
    ];
    if intended.is_none() {
        not_applicable.push(NotApplicable {
            rung: 5,
            name: "bound",
            reason: "no intended agent id, chain and registry were supplied to check the document's claim against",
        });
        not_applicable.sort_by_key(|n| n.rung);
    }

    Ok(Json(ValidateResponse {
        checker_version: checks::CHECKER_VERSION,
        schema_version: checks::SCHEMA_VERSION,
        spec_commit: checks::SPEC_COMMIT,
        body_bytes: body.len(),
        body_sha256,
        rungs: rungs
            .into_iter()
            .map(|r| ValidateRung {
                rung: r.rung,
                name: r.name,
                status: r.status.as_str().to_string(),
                evidence: r.evidence,
                checked_at: r.checked_at,
            })
            .collect(),
        not_applicable,
    }))
}
