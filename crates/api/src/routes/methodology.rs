//! The measurement windows, served as data.
//!
//! The liveness window and the rot threshold are part of the published
//! methodology: "answered 100 of 120 probes in the last 30 days" is only
//! checkable if a reader knows what 30 refers to. These constants are the
//! single definition: if a threshold were restated in prose, it would become
//! a second definition that could silently stop matching the one the queries use.
//!
//! Publishing them via `/api/methodology` means the queries and any frontend
//! that renders them all use the same numbers.

use axum::Json;
use serde::Serialize;

use crate::error::ApiResult;

/// The constants behind the published facts.
#[derive(Debug, Serialize)]
pub struct Methodology {
    /// Days of probe history an `endpoint_liveness` fact covers.
    pub liveness_window_days: i64,
    /// Days without a resolving metadata fetch before a card counts as rotted.
    pub rot_after_days: i64,
}

/// `GET /api/methodology` — no database access; these are compile-time facts
/// about how we measure, not measurements.
pub async fn get() -> ApiResult<Json<Methodology>> {
    Ok(Json(Methodology {
        liveness_window_days: facts::LIVENESS_WINDOW_DAYS,
        rot_after_days: facts::ROT_AFTER_DAYS,
    }))
}
