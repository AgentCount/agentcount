//! The Seller Census (METHODOLOGY §10), served.
//!
//! ## Why this is a separate module and separate URLs
//!
//! It would have been less code to widen `runs`, `rates` and `findings` with a
//! `network`/`chain` switch and let one set of URLs serve both instruments.
//! That is exactly the merge this project must not make. The two censuses
//! count different populations — registered agents, and sellers advertising
//! paid resources — and the one failure mode that would discredit both is a
//! figure that silently blends them. Different tables, different vocabulary,
//! different unit; different routes.
//!
//! The response shapes deliberately RHYME with the registration census's
//! (`run_id` and provenance on every response, counts never rates, `Option`
//! rather than a coalesced zero) so a consumer can learn one and read the
//! other. They do not share a type.
//!
//! ## The one thing this module exists to get right
//!
//! **A rung nobody asked is not a rung that scored zero.** Sweep 1 attempted
//! rungs 1, 2, 3, 6 and 7 and deliberately did not attempt rung 4, which
//! spends real money and waits on a funded wallet; rung 5 is reserved by the
//! method. `seller_runs.rungs_attempted` records what was asked, and every
//! rung this module serves carries [`RungRates::attempted`] beside its counts.
//!
//! A UI that reads only the counts sees an empty rung 4 and has every reason
//! to render "0% delivered", which would be this census publishing a
//! catastrophic claim about other people's businesses that it never measured.
//! So the flag is not a convenience field — it is the difference between a
//! missing measurement and a damning one, and it is why `rungs` below lists
//! every rung the method defines rather than only the rungs with rows.

use axum::Json;
use axum::extract::{Path, State};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::AppState;
use crate::error::{ApiError, ApiResult};

/// One seller run, as `seller_runs` recorded it. No derived fields.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SellerRunRow {
    pub run_id: Uuid,
    /// The SETTLEMENT scope for this sweep, not the population's scope.
    ///
    /// The population is not scoped by network (METHODOLOGY §10.5): every
    /// seller in every catalog is enumerated and asked whether it answers,
    /// quotes and matches its listing, whatever chain it settles on. This
    /// column says which chain rungs 6 and 4 could read. A consumer that
    /// prints it as "the sellers on Base" is wrong, and the field name is the
    /// only warning this API can give it.
    pub network: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// `running`, `finished` or `failed`. Served for the reason the
    /// registration census serves it: a failed sweep is stamped with the
    /// moment it died, so from outside it is indistinguishable from a
    /// complete one. Comparing against a crashed run is how the first seller
    /// delta reported 2,387 sellers "appeared".
    pub status: String,
    /// NULL until the sweep finishes. Never coalesced to 0.
    pub seller_count: Option<i32>,
    /// Which rungs this sweep set out to ask. NULL predates the column
    /// (migration 0027) and means "unrecorded", which is not the same as
    /// "none" — see [`RungRates::attempted`].
    pub rungs_attempted: Option<Vec<i16>>,
    pub catalogs: Vec<String>,
    pub seller_checker_version: String,
    pub checker_commit: String,
    pub rerun_command: Option<String>,
}

/// `GET /api/seller-runs` — every seller run, newest first, with provenance.
pub async fn list_runs(State(state): State<AppState>) -> ApiResult<Json<Vec<SellerRunRow>>> {
    let rows = sqlx::query_as::<_, SellerRunRow>(
        "SELECT run_id, network, started_at, finished_at, status, seller_count, \
                rungs_attempted, catalogs, seller_checker_version, checker_commit, \
                rerun_command \
         FROM seller_runs \
         ORDER BY started_at DESC",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

/// One status's count within one rung.
#[derive(Debug, Serialize)]
pub struct StatusCount {
    pub status: String,
    pub count: i64,
}

/// One rung's full status breakdown.
#[derive(Debug, Serialize)]
pub struct RungRates {
    pub rung: i16,
    /// The census's own name for the rung (`listed`, `reachable`, `quotes`,
    /// `delivers`, `settled`, `consistent`), so a UI labels the ladder from
    /// the data rather than hard-coding a vocabulary that can drift.
    ///
    /// Present even for a rung with no rows, which is the whole point: a rung
    /// that was never attempted still has a name and still has to be
    /// nameable by whatever renders it.
    pub name: &'static str,
    /// Whether this sweep set out to ask this rung.
    ///
    /// `false` means every count below is zero because nobody asked — NOT
    /// because every seller failed. Read this before reading `counts`. When
    /// the run predates `rungs_attempted` (NULL), this is `None`: unrecorded,
    /// which is a third thing and must not be flattened into either.
    pub attempted: Option<bool>,
    /// Why a rung might not have been attempted, when the method says so
    /// rather than the operator. `None` for rungs that were attempted.
    pub reserved: Option<&'static str>,
    pub counts: Vec<StatusCount>,
}

#[derive(Debug, Serialize)]
pub struct SellerRatesResponse {
    pub run_id: Uuid,
    /// How many sellers this run enumerated — the population size, the same
    /// number for every rung. Not a pass count and not a score.
    pub seller_count: i64,
    /// Distinct hosts behind those sellers. Published beside the seller count
    /// because the unit is `(payTo, host)` and the two differ a lot: one host
    /// quoting several payment addresses is several sellers, and a reader
    /// given only one of these numbers will assume the other equals it.
    pub host_count: i64,
    /// DISTINCT advertised resource URLs across the population — how large
    /// the advertised economy actually is.
    ///
    /// Distinct, because a resource can be advertised by more than one
    /// seller: the first run holds 22,289 seller-resource pairs over 14,739
    /// distinct URLs, so summing the per-seller arrays overstates the
    /// advertised economy by half. Both numbers are real and they answer
    /// different questions, so both are served rather than one being picked —
    /// see [`SellerRatesResponse::seller_resource_pairs`].
    pub resource_count: i64,
    /// Seller-resource pairs: the sum of every seller's own resource list.
    ///
    /// The right denominator for "how much work did this sweep enumerate",
    /// and the wrong one for "how many resources are for sale". Named for
    /// what it counts so the two can never be swapped by accident.
    pub seller_resource_pairs: i64,
    pub rungs: Vec<RungRates>,
}

/// Every rung the method defines, with the name the data uses.
///
/// Rung 5 is absent because it is absent from the method: `receipted` is
/// designed and deliberately outside the locked ladder until the x402
/// offers/receipts extension stabilises, and a rung that the census does not
/// define must not appear as one it merely did not run. The `CHECK` on
/// `seller_check_results.rung` encodes the same set.
const LADDER: &[(i16, &str, Option<&str>)] = &[
    (1, "listed", None),
    (2, "reachable", None),
    (3, "quotes", None),
    (
        4,
        "delivers",
        Some("spends real money; runs once the shopper wallet is funded"),
    ),
    (6, "settled", None),
    (7, "consistent", None),
];

/// Join the ladder the method defines to the counts a run actually produced.
///
/// Pure, and separated from the handler for one reason: this is where "nobody
/// asked" and "everybody failed" are told apart, and that distinction is worth
/// a test that does not need a database. Three cases, all real:
///
/// * a rung in `attempted` with rows — an ordinary measured rung;
/// * a rung NOT in `attempted` — never asked, counts empty, and a UI must not
///   render 0%. Rung 4 in every sweep so far;
/// * a rung in `attempted` with NO rows — asked, and nothing came back. Rare
///   but not impossible (an empty population), and it is `attempted: true`
///   with empty counts, which is a different claim from the case above.
///
/// `attempted: None` when the run predates `rungs_attempted` (migration 0027):
/// unrecorded, which is a third state and is never flattened into `false`.
fn assemble_rungs(attempted: Option<&[i16]>, counted: &[(i16, String, i64)]) -> Vec<RungRates> {
    LADDER
        .iter()
        .map(|&(rung, name, reserved)| RungRates {
            rung,
            name,
            attempted: attempted.map(|asked| asked.contains(&rung)),
            reserved,
            counts: counted
                .iter()
                .filter(|(r, _, _)| *r == rung)
                .map(|(_, status, count)| StatusCount {
                    status: status.clone(),
                    count: *count,
                })
                .collect(),
        })
        .collect()
}

/// `GET /api/seller-runs/{id}/rates` — per-rung status counts for one run.
///
/// `idx_seller_check_results_rates` (run_id, rung, status) exists for this
/// query.
pub async fn rates(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> ApiResult<Json<SellerRatesResponse>> {
    // The run first, and not merely to 404: `rungs_attempted` is read from it,
    // and without that every rung below would be served as a bare zero.
    let run = sqlx::query_as::<_, SellerRunRow>(
        "SELECT run_id, network, started_at, finished_at, status, seller_count, \
                rungs_attempted, catalogs, seller_checker_version, checker_commit, \
                rerun_command \
         FROM seller_runs WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    // The population, counted from the rows rather than read from
    // `seller_runs.seller_count`: that column is NULL for a run still in
    // flight, and these three numbers must agree with each other.
    let (seller_count, host_count, seller_resource_pairs): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*), count(DISTINCT host), \
                coalesce(sum(cardinality(resources)), 0) \
         FROM seller_population WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&state.db)
    .await?;

    // Counted separately because it cannot be derived from the row above: a
    // resource advertised by two sellers is two pairs and one URL, and only
    // unnesting can tell them apart.
    let resource_count: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT u) FROM seller_population p, unnest(p.resources) AS u \
         WHERE p.run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&state.db)
    .await?;

    #[derive(sqlx::FromRow)]
    struct Row {
        rung: i16,
        status: String,
        count: i64,
    }
    let raw = sqlx::query_as::<_, Row>(
        "SELECT rung, status, count(*) AS count FROM seller_check_results \
         WHERE run_id = $1 GROUP BY rung, status ORDER BY rung, status",
    )
    .bind(run_id)
    .fetch_all(&state.db)
    .await?;

    let counted: Vec<(i16, String, i64)> = raw
        .into_iter()
        .map(|r| (r.rung, r.status, r.count))
        .collect();
    let rungs = assemble_rungs(run.rungs_attempted.as_deref(), &counted);

    Ok(Json(SellerRatesResponse {
        run_id,
        seller_count,
        host_count,
        resource_count,
        seller_resource_pairs,
        rungs,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rung(rungs: &[RungRates], n: i16) -> &RungRates {
        rungs.iter().find(|r| r.rung == n).expect("rung in ladder")
    }

    /// THE test this module exists for.
    ///
    /// Sweep 1 asked rungs 1, 2, 3, 6 and 7. Rung 4 spends real money and was
    /// deliberately not asked. Served as a bare zero it reads "0% of sellers
    /// delivered what you paid for", which is a ruinous claim about other
    /// people's businesses that this census has never measured.
    #[test]
    fn a_rung_nobody_asked_is_not_a_rung_that_scored_zero() {
        let counted = vec![
            (1i16, "pass".to_string(), 2387i64),
            (3, "pass".to_string(), 754),
            (3, "fail".to_string(), 1486),
        ];
        let rungs = assemble_rungs(Some(&[1, 2, 3, 6, 7]), &counted);

        let delivers = rung(&rungs, 4);
        assert_eq!(delivers.attempted, Some(false));
        assert!(delivers.counts.is_empty());
        assert!(
            delivers.reserved.is_some(),
            "a rung the method holds back says why"
        );

        // ...while a rung that WAS asked and produced nothing is a different
        // claim, and must not be confused with the one above.
        let reachable = rung(&rungs, 2);
        assert_eq!(reachable.attempted, Some(true));
        assert!(reachable.counts.is_empty());
    }

    #[test]
    fn an_unrecorded_ladder_is_neither_asked_nor_skipped() {
        // NULL `rungs_attempted` predates migration 0027. Flattening it to
        // `false` would claim of an older run that it deliberately declined
        // rungs it may well have asked.
        let rungs = assemble_rungs(None, &[(1i16, "pass".to_string(), 10i64)]);
        for r in &rungs {
            assert_eq!(r.attempted, None, "rung {} guessed at an answer", r.rung);
        }
        assert_eq!(rung(&rungs, 1).counts.len(), 1, "counts still served");
    }

    #[test]
    fn every_rung_the_method_defines_is_served_even_with_no_rows() {
        // A UI cannot render a ladder it is not sent. Rung 5 is absent
        // because the METHOD does not define it — reserved until the x402
        // receipts extension stabilises — which is different from a rung that
        // exists and went unasked.
        let rungs = assemble_rungs(Some(&[1]), &[]);
        let numbers: Vec<i16> = rungs.iter().map(|r| r.rung).collect();
        assert_eq!(numbers, vec![1, 2, 3, 4, 6, 7]);
        assert!(!numbers.contains(&5));
    }
}
