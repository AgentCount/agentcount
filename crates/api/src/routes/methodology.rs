//! The measurement methodology, served as data.
//!
//! `spec_commit`, `checker_version`, and `schema_version` are the same
//! provenance constants stamped onto every run — published here too so a
//! reader can check "which spec commit does the checker I'd be running right
//! now judge against?" without opening a specific run. The rung-4
//! required-field list is re-exported from `checks` itself (not restated) so
//! this endpoint can never silently drift from what rung 4 actually checks.
//!
//! Replaces the retired liveness-window / rot-threshold methodology entirely
//! — those measured availability, which this product no longer publishes.

use axum::Json;
use serde::Serialize;

use crate::error::ApiResult;

/// One field rung 4 checks for presence, with whether it's unconditional
/// (checked on every document) or only required inside a `registrations`
/// entry when that array is present at all. See `spec/REQUIRED_FIELDS.md`
/// for the spec line citations behind each one.
#[derive(Debug, Serialize)]
pub struct RequiredField {
    pub field: String,
    pub condition: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Methodology {
    pub spec_commit: &'static str,
    pub checker_version: &'static str,
    pub schema_version: i32,
    pub rung4_required_fields: Vec<RequiredField>,
}

/// `GET /api/methodology` — no database access; these are compile-time facts
/// about how we measure, not measurements themselves.
pub async fn get() -> ApiResult<Json<Methodology>> {
    let mut rung4_required_fields: Vec<RequiredField> = checks::UNCONDITIONAL_FIELDS
        .iter()
        .map(|f| RequiredField {
            field: f.to_string(),
            condition: "unconditional",
        })
        .collect();
    rung4_required_fields.extend(checks::REGISTRATION_ENTRY_FIELDS.iter().map(|f| {
        RequiredField {
            field: format!("registrations[].{f}"),
            condition: "required within each entry, when `registrations` is present",
        }
    }));

    Ok(Json(Methodology {
        spec_commit: checks::SPEC_COMMIT,
        checker_version: checks::CHECKER_VERSION,
        schema_version: checks::SCHEMA_VERSION,
        rung4_required_fields,
    }))
}
