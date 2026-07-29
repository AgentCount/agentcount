//! The measurement methodology, served as data.
//!
//! `spec_commit`, `checker_version`, and `schema_version` are the same
//! provenance constants stamped onto every run — published here too so a
//! reader can check "which spec commit does the checker I'd be running right
//! now judge against?" without opening a specific run. The rung-4 field
//! lists are re-exported from `checks` itself (not restated) so this
//! endpoint can never silently drift from what rung 4 actually checks.
//!
//! **P0 FIX 3 — three severities, not one required list.** Rung 4 no longer
//! has a single flat "required fields" list: it has a MUST bucket (the only
//! thing that can fail the rung), a SHOULD bucket, and a MAY bucket. See
//! `spec/REQUIRED_FIELDS.md` for the full citation trail behind every
//! field's bucket.
//!
//! Replaces the retired liveness-window / rot-threshold methodology entirely
//! — those measured availability, which this product no longer publishes.

use axum::Json;
use serde::Serialize;

use crate::error::ApiResult;

/// One field, with the condition under which it's checked. Used for the
/// MUST bucket, where every entry is conditional on `registrations` being
/// present at all — there is no unconditional MUST field left after P0
/// FIX 3.
#[derive(Debug, Serialize)]
pub struct ConditionalField {
    pub field: String,
    pub condition: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Methodology {
    pub spec_commit: &'static str,
    pub checker_version: &'static str,
    pub schema_version: i32,
    /// The only fields whose absence fails rung 4. Always conditional
    /// (checked only inside `registrations[]` entries, only when that array
    /// is present) — see `spec/REQUIRED_FIELDS.md` §MUST.
    pub rung4_must_fields: Vec<ConditionalField>,
    /// Checked for presence; absence is recorded as a `should_gaps` entry in
    /// evidence but never fails the rung. See `spec/REQUIRED_FIELDS.md`
    /// §SHOULD. `services` (empty-vs-absent), `registrations` (at least
    /// one), and `services[].version` are handled specially — see the
    /// linked doc — but are listed here by name for completeness.
    pub rung4_should_fields: Vec<String>,
    /// Purely informational; absence never appears as anything but a
    /// `may_gaps` entry. See `spec/REQUIRED_FIELDS.md` §MAY.
    pub rung4_may_fields: Vec<String>,
}

/// `GET /api/methodology` — no database access; these are compile-time facts
/// about how we measure, not measurements themselves.
pub async fn get() -> ApiResult<Json<Methodology>> {
    let rung4_must_fields: Vec<ConditionalField> = checks::REGISTRATION_ENTRY_FIELDS
        .iter()
        .map(|f| ConditionalField {
            field: format!("registrations[].{f}"),
            condition: "required within each entry, only when `registrations` is present",
        })
        .collect();

    let rung4_should_fields: Vec<String> = checks::SHOULD_TOP_LEVEL_FIELDS
        .iter()
        .chain(checks::SHOULD_SPECIAL_FIELDS.iter())
        .map(|f| f.to_string())
        .collect();

    let rung4_may_fields: Vec<String> = checks::MAY_FIELDS.iter().map(|f| f.to_string()).collect();

    Ok(Json(Methodology {
        spec_commit: checks::SPEC_COMMIT,
        checker_version: checks::CHECKER_VERSION,
        schema_version: checks::SCHEMA_VERSION,
        rung4_must_fields,
        rung4_should_fields,
        rung4_may_fields,
    }))
}
