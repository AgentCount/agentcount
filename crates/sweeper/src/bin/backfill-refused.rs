//! # backfill-refused — re-judge every archived run under the `refused` rules.
//!
//! ```text
//! DATABASE_URL=… backfill-refused                    # every run, DRY RUN
//! DATABASE_URL=… backfill-refused --apply            # every run, for real
//! DATABASE_URL=… backfill-refused --apply <run-id>…  # only these runs
//! ```
//!
//! ## Why an archived run is rewritten at all
//!
//! This project does not rewrite measurements. It says so in `METHODOLOGY.md`
//! §5 and it has declined to reissue archives over worse defects than this one.
//! The exception here is narrow and worth stating precisely:
//!
//! **Nothing measured changed. A word changed.** The 2026-08-06 work did not
//! re-probe anything and did not decide any agent behaved differently. It
//! decided that HTTP 429/503/401/402/407, and a `robots.txt` we could not get
//! permission from, were being described by the wrong word — `fail`, which is
//! the agent's word, and `error`, which is ours. Every row this binary moves is
//! moved from evidence that is already in the row: the archived `http_status`
//! and `reason` this run recorded at the time. No network request is made and
//! no chain is read.
//!
//! **A series that says two things is not a series.** `stopped_resolving` is
//! computed between consecutive runs. If 2026-07 says `fail` for a 429 and
//! 2026-08 says `refused` for the same 429 at the same host, the delta between
//! them reports a flip that is entirely an artifact of when we changed our
//! minds. Leaving the old runs alone would have published exactly the kind of
//! false churn the `refused` status exists to prevent — see
//! `sweeper::delta`'s module doc.
//!
//! So: re-judge every run, restamp each one it touches to the checker version
//! and schema that produced the new judgment (`restamp_checker`, the same
//! mechanism the rung-6 pass already uses), and recompute every delta.
//!
//! ## What it does NOT do
//!
//! * **It does not touch rung 6.** Rung 6 gained `refused` in the same change,
//!   and re-judging it needs no bespoke code: `liveness <chain> <run-id>`
//!   re-reads the archived `endpoint_probes` rows for that run and re-judges
//!   every agent through `checks::live` itself, sending no new requests
//!   (already-probed URLs are skipped by design, because that pass is
//!   resumable). Running it here would mean a second implementation of rung 6's
//!   aggregation, which is exactly the kind of duplicate that eventually
//!   disagrees. **Run `liveness` for each run after this binary.**
//! * **It does not reissue published archives.** Each published archive holds
//!   the bytes that run exported at the time, and those keep the old words. The
//!   mapping is mechanical and total — see `DATA.md` and the 2026-08-06
//!   changelog entry — and an archive that verifies against a hash in a commit
//!   predating this change is more useful than one quietly rewritten.
//! * **It never moves a `pass`.** A row that passed is untouched, and no row
//!   becomes a `pass`. The only transitions it can make are `fail → refused`
//!   and `error → refused`.
//!
//! Dry run by default, because this writes to published data and a binary that
//! does that on a typo is a defect of its own. The dry run reports exactly what
//! `--apply` would do, per run.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use sweeper::store;
use uuid::Uuid;

/// How many agent ids go into one `UPDATE … agent_id = ANY($2)`.
///
/// The BSC runs move ~49,000 rows each. One statement with 49,000 parameters
/// is legal and unreadable in a log; chunking also means an interrupted run has
/// done a knowable amount, and this binary is safely re-runnable either way
/// (a row already `refused` is not a candidate the second time).
const CHUNK: usize = 5_000;

/// What this binary decided about one archived rung-2 row.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Move to `refused`, and rewrite the generic `http_status` reason to the
    /// `declined` the current checker would have written.
    RefusedDeclined,
    /// Move to `refused`, keeping the reason the row already carries —
    /// `payment_required`, or the verbatim `robots_*` text.
    RefusedKeepReason,
    /// Nothing to do.
    Unchanged,
}

/// Re-judge one archived row from its own evidence.
///
/// The predicates are `checks::refusal`'s, not copies of them: this binary and
/// the live checker have to agree about what a 429 is, and the only way to
/// guarantee that is to ask the same function.
fn rejudge(status: &str, evidence: &serde_json::Value) -> Verdict {
    let reason = evidence
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    match status {
        "fail" => match evidence.get("http_status").and_then(|v| v.as_u64()) {
            Some(code) if checks::refusal::declined_us(code as u16) => {
                // A 402 already carries `payment_required`, which the current
                // checker still writes. Everything else carried the generic
                // `http_status`.
                if reason == "http_status" {
                    Verdict::RefusedDeclined
                } else {
                    Verdict::RefusedKeepReason
                }
            }
            _ => Verdict::Unchanged,
        },
        // `robots_disallowed` / `robots_unavailable: …`, which used to be read
        // as this checker malfunctioning.
        "error" if checks::refusal::could_not_ask(reason) => Verdict::RefusedKeepReason,
        _ => Verdict::Unchanged,
    }
}

/// One run's before/after, as printed.
struct RunReport {
    run_id: Uuid,
    chain: String,
    before: BTreeMap<String, i64>,
    after: BTreeMap<String, i64>,
    from_fail: usize,
    from_error: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let apply = args.iter().any(|a| a == "--apply");
    let only: Vec<Uuid> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|a| Uuid::parse_str(a))
        .collect::<Result<_, _>>()
        .context("run ids must be uuids")?;

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = store::Db::connect(&database_url).await?;

    let runs: Vec<(Uuid, String)> = db
        .runs_with_results()
        .await?
        .into_iter()
        .filter(|(id, _)| only.is_empty() || only.contains(id))
        .collect();
    if runs.is_empty() {
        tracing::warn!("no runs with results matched — nothing to do");
        return Ok(());
    }
    if !apply {
        tracing::warn!("DRY RUN — nothing will be written. Re-run with --apply to reclassify.");
    }

    let mut reports = Vec::new();
    for (run_id, chain) in &runs {
        let before = counts(&db, *run_id).await?;

        let mut declined: Vec<i64> = Vec::new();
        let mut keep_reason: Vec<i64> = Vec::new();
        let mut from_fail = 0usize;
        let mut from_error = 0usize;
        for (agent_id, status, evidence) in db.rung2_candidates(*run_id).await? {
            match rejudge(&status, &evidence) {
                Verdict::RefusedDeclined => declined.push(agent_id),
                Verdict::RefusedKeepReason => keep_reason.push(agent_id),
                Verdict::Unchanged => continue,
            }
            match status.as_str() {
                "fail" => from_fail += 1,
                _ => from_error += 1,
            }
        }

        if apply {
            for chunk in declined.chunks(CHUNK) {
                db.mark_rung2_refused(*run_id, chain, chunk, true).await?;
            }
            for chunk in keep_reason.chunks(CHUNK) {
                db.mark_rung2_refused(*run_id, chain, chunk, false).await?;
            }
            if !declined.is_empty() || !keep_reason.is_empty() {
                // The run has now been judged by this checker, and must say so
                // — a run whose rows use a vocabulary its stamp predates is
                // unciteable.
                db.restamp_checker(*run_id, checks::SCHEMA_VERSION, checks::CHECKER_VERSION)
                    .await?;
            }
        }

        let after = if apply {
            counts(&db, *run_id).await?
        } else {
            projected(&before, declined.len() + keep_reason.len(), from_error)
        };
        reports.push(RunReport {
            run_id: *run_id,
            chain: chain.clone(),
            before,
            after,
            from_fail,
            from_error,
        });
    }

    // ── The report. This is the deliverable, not a progress log ──────────
    println!(
        "\nrung 2 — before → after, per run{}",
        if apply {
            ""
        } else {
            "  (PROJECTED — dry run)"
        }
    );
    println!(
        "{:<38} {:<8} {:>10} {:>10} {:>10} {:>10}",
        "run", "chain", "pass", "fail", "error", "refused"
    );
    for r in &reports {
        println!(
            "{:<38} {:<8} {:>10} {:>10} {:>10} {:>10}",
            r.run_id,
            r.chain,
            fmt(&r.before, &r.after, "pass"),
            fmt(&r.before, &r.after, "fail"),
            fmt(&r.before, &r.after, "error"),
            fmt(&r.before, &r.after, "refused"),
        );
        println!(
            "{:<38} {:<8} → refused: {} from fail (rate limits, challenges), \
             {} from error (robots.txt)",
            "", "", r.from_fail, r.from_error
        );
    }

    if !apply {
        println!("\nDRY RUN — nothing was written. Re-run with --apply.");
        return Ok(());
    }

    // ── Deltas, which are derived and therefore legitimately recomputed ──
    let recomputed = sweeper::recompute::all(&db, true).await?;
    tracing::info!("recomputed {} delta row(s)", recomputed.len());
    for r in &recomputed {
        println!(
            "delta {} {}: -{} stopped resolving, +{} newly resolving",
            r.chain, r.run_id, r.counts.stopped_resolving, r.counts.newly_resolving
        );
    }

    println!(
        "\nDone. Rung 6 is NOT re-judged here — run `liveness <chain> <run-id>` \
         for each run above; it re-reads the archived probes and sends no requests."
    );
    Ok(())
}

async fn counts(db: &store::Db, run_id: Uuid) -> Result<BTreeMap<String, i64>> {
    Ok(db
        .rung_status_counts(run_id, 2)
        .await?
        .into_iter()
        .collect())
}

/// What `--apply` would produce, for the dry run's report.
fn projected(
    before: &BTreeMap<String, i64>,
    moved: usize,
    from_error: usize,
) -> BTreeMap<String, i64> {
    let mut after = before.clone();
    let from_fail = moved - from_error;
    *after.entry("fail".into()).or_insert(0) -= from_fail as i64;
    *after.entry("error".into()).or_insert(0) -= from_error as i64;
    *after.entry("refused".into()).or_insert(0) += moved as i64;
    after
}

fn fmt(before: &BTreeMap<String, i64>, after: &BTreeMap<String, i64>, status: &str) -> String {
    let b = before.get(status).copied().unwrap_or(0);
    let a = after.get(status).copied().unwrap_or(0);
    if a == b {
        format!("{b}")
    } else {
        format!("{b}→{a}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_429_recorded_as_fail_becomes_refused_and_gains_the_new_reason() {
        let ev = json!({"http_status": 429, "reason": "http_status", "scheme": "https"});
        assert_eq!(rejudge("fail", &ev), Verdict::RefusedDeclined);
    }

    #[test]
    fn a_402_keeps_the_reason_it_already_had() {
        // `payment_required` is what the current checker writes too, so
        // rewriting it would make the backfilled row differ from a fresh one.
        let ev = json!({"http_status": 402, "reason": "payment_required"});
        assert_eq!(rejudge("fail", &ev), Verdict::RefusedKeepReason);
    }

    #[test]
    fn the_robots_errors_become_refused_with_their_reason_intact() {
        for reason in [
            "robots_disallowed",
            "robots_unavailable: connection failed fetching robots.txt: os error 54",
            "robots_unavailable: robots.txt returned HTTP 503",
        ] {
            let ev = json!({"reason": reason});
            assert_eq!(
                rejudge("error", &ev),
                Verdict::RefusedKeepReason,
                "{reason}"
            );
        }
    }

    #[test]
    fn nothing_else_moves() {
        // The statuses that stay put, and the failures that are still the
        // agent's. A backfill that moved any of these would be rewriting a
        // measurement rather than renaming one.
        for (status, ev) in [
            ("fail", json!({"http_status": 404, "reason": "http_status"})),
            ("fail", json!({"http_status": 403, "reason": "http_status"})),
            ("fail", json!({"http_status": 500, "reason": "http_status"})),
            ("fail", json!({"reason": "no_uri"})),
            (
                "fail",
                json!({"reason": "ssrf_blocked: dns resolution failed"}),
            ),
            ("error", json!({"reason": "timeout"})),
            ("error", json!({"reason": "ipfs_all_gateways_failed"})),
            ("error", json!({"reason": "unsupported_compression: zstd"})),
            ("pass", json!({"http_status": 200})),
            ("skipped", json!({"skipped_because_rung": 1})),
        ] {
            assert_eq!(rejudge(status, &ev), Verdict::Unchanged, "{status} {ev}");
        }
    }

    #[test]
    fn a_row_already_refused_is_never_a_candidate_so_this_is_re_runnable() {
        // `rung2_candidates` only reads `fail`/`error`, and neither verdict
        // above can produce anything but `refused` — so a second run finds
        // nothing left to do rather than double-counting.
        let ev = json!({"http_status": 429, "reason": "declined"});
        assert_eq!(rejudge("refused", &ev), Verdict::Unchanged);
    }

    #[test]
    fn the_projection_matches_what_apply_would_leave_behind() {
        let before: BTreeMap<String, i64> = [
            ("pass".to_string(), 180_825),
            ("fail".to_string(), 62_515),
            ("error".to_string(), 868),
        ]
        .into_iter()
        .collect();
        // The measured 2026-07-29 BSC numbers: 48,642 from fail, 737 from
        // error, 49,379 refused in total.
        let after = projected(&before, 49_379, 737);
        assert_eq!(after["pass"], 180_825);
        assert_eq!(after["fail"], 13_873);
        assert_eq!(after["error"], 131);
        assert_eq!(after["refused"], 49_379);
        let total_before: i64 = before.values().sum();
        let total_after: i64 = after.values().sum();
        assert_eq!(
            total_before, total_after,
            "reclassification moves rows between statuses; it never creates or drops one"
        );
    }
}
