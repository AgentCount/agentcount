//! # liveness — rung 6 (`live`), as a second pass over a finished run.
//!
//! ```text
//! DATABASE_URL=… liveness base                       # newest finished run
//! DATABASE_URL=… liveness base 7833fc49-…            # a specific run
//! ```
//!
//! Reads the documents a run already archived, works out what service
//! endpoints they declared, probes the http(s) ones, and writes each agent's
//! rung-6 row. It makes no chain calls and re-reads no document over the
//! network — everything it needs about the agents is already in the database.
//!
//! ## Why it is a pass and not a pipeline stage
//!
//! The main sweep's unit of work is an agent. Rung 6's is a URL, and the two
//! do not line up: 125,705 declared HTTP(S) endpoints across the census resolve
//! to 3,399 distinct hosts, with four hosts carrying 59.2% of them. Both
//! deduplication and the per-host budget need the whole population in hand
//! before the first request goes out, which a per-agent pipeline cannot give.
//!
//! It also means the probe — the slow, rate-limited, interruptible half — can
//! be re-run without re-reading a chain.
//!
//! ## Four properties this binary is built around
//!
//! * **Resumable.** `endpoint_probes` IS the checkpoint. A pass that dies at
//!   hour two resumes by reading what landed, and sends no request it has
//!   already sent. There is no state file to go stale.
//! * **Deterministic.** The sample is chosen by a fixed hash of the URL, not
//!   by arrival order or by chance, so the same run and the same budget
//!   select the same URLs — on a resume, on a re-run, and on someone else's
//!   machine. A sample nobody can reproduce is not evidence.
//! * **Polite.** One request per distinct URL, `crates/probe`'s robots.txt
//!   and SSRF discipline unchanged, its per-host concurrency cap unchanged,
//!   and a per-host budget above all of it.
//! * **Honest about what it did not do.** An agent whose every URL fell
//!   outside the budget gets NO rung-6 row. See `checks::live`'s module doc.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use chrono::Utc;
use futures::stream::{self, StreamExt};
use sweeper::store;
use uuid::Uuid;

/// Distinct URLs probed per host before the rest are left alone.
///
/// 500 is enough to put a tight interval on a host's live rate — at that size
/// the sampling error on a proportion is under ±4.5 points at 95% confidence,
/// whatever the host's true rate — while capping the largest operator in the
/// census at 500 requests instead of 26,273. Hosts below the budget are
/// probed in full, which is 3,391 of the 3,399.
fn host_budget() -> usize {
    std::env::var("RUNG6_HOST_BUDGET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500)
}

/// How many probes are in flight at once, across all hosts.
///
/// `crates/probe` enforces its own per-host cap underneath this, so raising it
/// widens the sweep across hosts rather than deepening it against any one of
/// them.
fn probe_concurrency() -> usize {
    std::env::var("RUNG6_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(probe::DEFAULT_GLOBAL_CONCURRENCY)
}

/// Where the probe's User-Agent points a server operator who wants to know who
/// just called. It must resolve and the mailbox must answer — rung 6 waited on
/// exactly that, and sending 100,000 requests behind a dead contact address
/// would be the rudest thing this project could do.
const PROBE_CONTACT_URL: &str = "https://agentcount.ai/methodology";

/// FNV-1a, written out rather than taken from `DefaultHasher`.
///
/// The sample has to be reproducible by anyone, years later, from the URL list
/// alone. `std::collections::hash_map::DefaultHasher` explicitly does not
/// promise a stable algorithm across Rust releases, so a sample chosen with it
/// would silently become a different sample after a toolchain upgrade — and
/// the run that published a rate would no longer be re-derivable. This is
/// eleven lines and fixed forever.
fn fnv1a(s: &str) -> u64 {
    // Offset basis and prime, both from the FNV spec. The prime is
    // 0x100000001b3 — grouped as `100_0000_01b3`, which is one digit-group
    // away from `1000_0000_01b3`, a different number entirely. Writing it
    // wrong still produces a perfectly deterministic hash, so the only thing
    // that catches it is a test against the published vectors, which is why
    // `the_sample_is_the_same_every_time_and_on_every_machine` exists.
    const BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x100_0000_01b3;
    let mut h = BASIS;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// The host part of a URL, lowercased, or `None` if there isn't one.
///
/// Hand-rolled rather than pulled from a URL crate: this crate already avoids
/// re-parsing what `crates/probe` parses, and the only thing needed here is a
/// grouping key for the budget. A string this cannot find a host in is not
/// probeable anyway — `crates/probe`'s netguard will reject it — so it is
/// grouped under the empty host and shares one budget with the other
/// malformed ones.
fn host_of(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    // Strip userinfo. `user:pass@host:443` → `host:443`.
    let authority = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    // Then the port. A bracketed IPv6 literal is handled first and separately:
    // `[2001:db8::1]:443` is full of colons that are not a port separator, and
    // the only one that is comes after the closing bracket. Trying to tell
    // them apart by counting colons gets this wrong in both directions.
    let host = if let Some(rest) = authority.strip_prefix('[') {
        rest.split_once(']').map(|(h, _)| h).unwrap_or(rest)
    } else {
        match authority.rsplit_once(':') {
            Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
            _ => authority,
        }
    };
    host.to_ascii_lowercase()
}

/// One agent's declared endpoints, pulled out of its archived document.
///
/// Not carrying the agent id: every collection of these is keyed by it, and a
/// copy inside the value is one more thing that can disagree with the key.
struct Declared {
    /// `(name, endpoint)` per entry, in declared order. `endpoint` is `None`
    /// when the entry carried no such field.
    entries: Vec<(Option<String>, Option<String>)>,
}

/// Read `services` — or the legacy `endpoints` alias — out of a document.
///
/// The alias rule is rung 4's, deliberately not re-derived: `services` wins
/// when both are present. Duplicating "which field counts" would let rung 4
/// and rung 6 disagree about whether an agent declared anything at all.
fn declared_endpoints(body: &[u8]) -> Option<Declared> {
    let doc: serde_json::Value = serde_json::from_slice(body).ok()?;
    let services = doc
        .get("services")
        .filter(|v| !v.is_null())
        .or_else(|| doc.get("endpoints").filter(|v| !v.is_null()))?;
    let array = services.as_array()?;
    let entries = array
        .iter()
        .map(|e| {
            let name = e.get("name").and_then(|v| v.as_str()).map(str::to_string);
            // Only a STRING endpoint is an endpoint. A number or an object
            // there is not a URL, and coercing it would invent a claim the
            // document did not make — `classify_endpoint(None)` reads it as
            // `missing`, which is what it is for our purposes.
            let endpoint = e
                .get("endpoint")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            (name, endpoint)
        })
        .collect();
    Some(Declared { entries })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let chain = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "base".to_string());
    let explicit_run = std::env::args()
        .nth(2)
        .map(|s| Uuid::parse_str(&s))
        .transpose()?;

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let db = store::Db::connect(&database_url).await?;

    let run_id = match explicit_run {
        Some(id) => id,
        None => {
            db.latest_finished_run(&chain)
                .await?
                .with_context(|| format!("no finished run for chain {chain}"))?
                .0
        }
    };
    tracing::info!("rung 6 pass over run {run_id} ({chain})");

    // ── 1. What every agent declared ─────────────────────────────────────
    let candidates = db.rung6_candidates(run_id).await?;
    tracing::info!("{} agents in this run", candidates.len());

    let mut declared_by_agent: HashMap<u64, Declared> = HashMap::new();
    // Distinct URL → how many declared entries across the run resolve to it.
    // The count is the extrapolation weight and goes into `endpoint_probes`.
    let mut url_weight: HashMap<String, usize> = HashMap::new();

    for c in &candidates {
        // Rung 6 depends on rung 4. An agent whose rung 4 did not pass is
        // handled below by `run_ladder`, and its document is not read here —
        // asking a server about a document that never conformed would be a
        // request nobody needed.
        if c.rung4_status != "pass" {
            continue;
        }
        let Some(body) = &c.body else { continue };
        let Some(d) = declared_endpoints(body) else {
            continue;
        };
        for (_, endpoint) in &d.entries {
            if checks::classify_endpoint(endpoint.as_deref()).is_probeable() {
                // `trim` and nothing else. Ruling 3 says exact URLs, and any
                // further normalisation (a trailing slash, case-folding a
                // path) would merge two URLs a server is free to treat as
                // different — and would make the dedupe count unreproducible
                // by anyone who normalised even slightly differently.
                let url = endpoint.as_deref().unwrap_or_default().trim().to_string();
                *url_weight.entry(url).or_insert(0) += 1;
            }
        }
        declared_by_agent.insert(c.agent_id, d);
    }

    let total_declared: usize = url_weight.values().sum();
    tracing::info!(
        "{} probeable entries declared by {} rung-4-passing agents, \
         {} distinct URLs after dedupe",
        total_declared,
        declared_by_agent.len(),
        url_weight.len(),
    );

    // ── 2. The sample ────────────────────────────────────────────────────
    let budget = host_budget();
    let mut by_host: HashMap<String, Vec<String>> = HashMap::new();
    for url in url_weight.keys() {
        by_host.entry(host_of(url)).or_default().push(url.clone());
    }

    let mut selected: Vec<(String, String)> = Vec::new(); // (url, host)
    let mut sampled_hosts = 0usize;
    let mut dropped = 0usize;
    for (host, mut urls) in by_host {
        if urls.len() > budget {
            sampled_hosts += 1;
            dropped += urls.len() - budget;
            // Deterministic and structure-blind: lexicographic order would
            // take every `…/agent/0000…` and miss the rest of the host's URL
            // space entirely, which is a biased sample dressed as a simple
            // one. FNV-1a of the URL spreads the selection across it and is
            // reproducible forever — see `fnv1a`.
            urls.sort_by_key(|u| (fnv1a(u), u.clone()));
            urls.truncate(budget);
        } else {
            urls.sort();
        }
        for u in urls {
            selected.push((u, host.clone()));
        }
    }
    // A stable order for the probe itself, so a resumed pass proceeds through
    // the same list in the same order and its logs line up with the first
    // attempt's.
    selected.sort();

    if sampled_hosts > 0 {
        tracing::warn!(
            "{sampled_hosts} host(s) exceeded the {budget}-URL budget; \
             {dropped} distinct URLs will NOT be probed and their agents will get \
             no rung-6 row"
        );
    }

    // ── 3. Probe, skipping whatever a previous attempt already did ───────
    let already = db.probed_urls(run_id).await?;
    if !already.is_empty() {
        tracing::info!("resuming: {} URLs already probed", already.len());
    }
    let todo: Vec<(String, String)> = selected
        .iter()
        .filter(|(u, _)| !already.contains_key(u))
        .cloned()
        .collect();
    tracing::info!("{} URLs to probe", todo.len());

    let gateways: Vec<String> = Vec::new();
    // No IPFS gateways: ruling 2 probes http(s) only, so the gateway chain is
    // unreachable from here and an empty list says that rather than carrying
    // three URLs nothing will use.
    let prober = probe::Prober::new(PROBE_CONTACT_URL, &gateways)?;
    let concurrency = probe_concurrency();
    let db_ref = &db;
    let prober_ref = &prober;
    let done = std::sync::atomic::AtomicUsize::new(0);
    let done_ref = &done;
    let total_todo = todo.len();

    stream::iter(todo)
        .map(|(url, host)| async move {
            let outcome = prober_ref.fetch(&url).await;
            (url, host, outcome)
        })
        .buffer_unordered(concurrency)
        .for_each(|(url, host, outcome)| {
            let url_weight = &url_weight;
            async move {
                let weight = *url_weight.get(&url).unwrap_or(&1) as i32;
                // A declared endpoint is an attacker-controlled string and
                // reaches a TEXT column here, exactly like `agent_uri` does in
                // the main sweep.
                let safe_url = store::escape_nuls_for_postgres(0, &url);
                if let Err(e) = db_ref
                    .record_probe(run_id, &safe_url, &host, weight, &outcome)
                    .await
                {
                    // One unwritable row must not cost the pass. It simply
                    // stays unprobed, and its agents get no rung-6 row —
                    // which is the same honest outcome as never reaching it.
                    tracing::warn!("could not record probe of {url}: {e}");
                }
                let n = done_ref.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if n.is_multiple_of(500) {
                    tracing::info!("probed {n}/{total_todo}");
                }
            }
        })
        .await;

    // ── 4. Judge ─────────────────────────────────────────────────────────
    let observations = db.probed_urls(run_id).await?;
    let selected_urls: HashSet<&String> = selected.iter().map(|(u, _)| u).collect();
    let now = Utc::now();

    let mut written = 0usize;
    let mut absent = 0usize;
    let mut by_status: HashMap<&'static str, usize> = HashMap::new();

    for c in &candidates {
        // The rung-6 candidate result, before the ladder gets a say.
        let candidate = declared_by_agent.get(&c.agent_id).map(|d| {
            let mut host_budget_reached = false;
            let endpoints: Vec<checks::ServiceEndpoint> = d
                .entries
                .iter()
                .enumerate()
                .map(|(index, (name, endpoint))| {
                    let probeable = checks::classify_endpoint(endpoint.as_deref()).is_probeable();
                    let url = endpoint
                        .as_deref()
                        .map(|e| e.trim().to_string())
                        .filter(|_| probeable);
                    let observed = url.as_ref().and_then(|u| observations.get(u)).map(|p| {
                        checks::EndpointObservation {
                            http_status: p.http_status,
                            error: p.error.clone(),
                            elapsed_ms: p.elapsed_ms,
                            final_url: p.final_url.clone(),
                        }
                    });
                    if probeable
                        && observed.is_none()
                        && url.as_ref().is_some_and(|u| !selected_urls.contains(u))
                    {
                        host_budget_reached = true;
                    }
                    checks::ServiceEndpoint {
                        index,
                        name: name.clone(),
                        declared: endpoint.clone(),
                        probed_url: url,
                        observed,
                    }
                })
                .collect();
            checks::live(
                &checks::LiveInput {
                    endpoints,
                    host_budget_reached,
                },
                now,
            )
        });

        // An agent that never reached rung 4 has no `declared_by_agent` entry,
        // and its rung 6 is `Skipped` — but this binary does NOT decide that.
        // `checks::run_ladder` is the one place skip-propagation is decided,
        // and calling it with rung 4 and a rung-6 candidate is enough, because
        // rung 6 depends on rung 4 alone.
        let laddered = match candidate {
            // Probeable endpoints, none probed: no row at all. Not a status.
            Some(None) => {
                if let Err(e) = db.clear_rung6(run_id, c.agent_id).await {
                    tracing::warn!("could not clear rung 6 for {}: {e}", c.agent_id);
                }
                absent += 1;
                continue;
            }
            Some(Some(r)) => {
                let out = checks::run_ladder(vec![rung4_as_read(&c.rung4_status, now), r]);
                out.into_iter().find(|x| x.rung == 6)
            }
            None => {
                // No document to read services from. If rung 4 passed, this is
                // a hole in our own data rather than a fact about the agent —
                // leave rung 6 absent rather than invent a verdict. If rung 4
                // did not pass, the ladder's `Skipped` is the right row.
                if c.rung4_status == "pass" {
                    absent += 1;
                    continue;
                }
                let placeholder = checks::CheckResult {
                    rung: 6,
                    name: "live",
                    status: checks::CheckStatus::Unprobeable,
                    evidence: serde_json::json!({}),
                    checked_at: now,
                };
                let out =
                    checks::run_ladder(vec![rung4_as_read(&c.rung4_status, now), placeholder]);
                out.into_iter().find(|x| x.rung == 6)
            }
        };

        let Some(mut result) = laddered else { continue };
        // Same `jsonb` hazard as every other evidence write: a declared
        // endpoint may carry a NUL, and Postgres rejects one in `jsonb`.
        store::escape_nuls_in_json(&mut result.evidence);
        *by_status.entry(result.status.as_str()).or_insert(0) += 1;
        if let Err(e) = db.replace_rung6(run_id, &chain, c.agent_id, &result).await {
            tracing::warn!("could not write rung 6 for {}: {e}", c.agent_id);
            continue;
        }
        written += 1;
    }

    db.restamp_checker(run_id, checks::SCHEMA_VERSION, checks::CHECKER_VERSION)
        .await?;

    let mut summary: Vec<String> = by_status.iter().map(|(s, n)| format!("{s} {n}")).collect();
    summary.sort();
    tracing::info!(
        "rung 6 complete for run {run_id}: {written} rows written ({}), \
         {absent} agents left with no row",
        summary.join(", ")
    );
    Ok(())
}

/// Rebuild rung 4's result from the status string the sweep stored.
///
/// Only the status is reconstructed — `run_ladder` reads nothing else from a
/// dependency, and inventing evidence for a row we did not write would put a
/// second, fabricated copy of rung 4 into this process.
fn rung4_as_read(status: &str, now: chrono::DateTime<Utc>) -> checks::CheckResult {
    let status = match status {
        "pass" => checks::CheckStatus::Pass,
        "fail" => checks::CheckStatus::Fail,
        "error" => checks::CheckStatus::Error,
        "unclaimed" => checks::CheckStatus::Unclaimed,
        "unprobeable" => checks::CheckStatus::Unprobeable,
        // `skipped`, and the empty string standing for a rung-4 row that does
        // not exist. Both mean the same thing to rung 6: rung 4 did not pass,
        // so there was nothing to probe from.
        _ => checks::CheckStatus::Skipped,
    };
    checks::CheckResult {
        rung: 4,
        name: "conformant",
        status,
        evidence: serde_json::Value::Null,
        checked_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_of_strips_scheme_path_port_and_userinfo() {
        for (url, want) in [
            ("https://example.com/a2a", "example.com"),
            ("https://Example.COM/A2A", "example.com"),
            ("https://example.com:8443/x", "example.com"),
            ("http://user:pass@example.com/x", "example.com"),
            ("https://example.com", "example.com"),
            ("https://example.com?q=1", "example.com"),
            ("https://[2001:db8::1]:443/x", "2001:db8::1"),
            ("https://[2001:db8::1]/x", "2001:db8::1"),
            // A colon that is not a port separator must not eat the host.
            ("https://example.com:notaport/x", "example.com:notaport"),
            // Nothing recognisable. Grouped under one budget rather than each
            // getting its own — see the doc comment.
            ("not a url at all", "not a url at all"),
        ] {
            assert_eq!(host_of(url), want, "{url}");
        }
    }

    #[test]
    fn the_sample_is_the_same_every_time_and_on_every_machine() {
        // The whole claim of a disclosed sample is that someone else can take
        // it again and get the same URLs. Two things could break that
        // silently, and these vectors catch both: a hash whose output moves
        // between Rust releases, and this implementation not being the FNV-1a
        // it says it is. The three values are the published test vectors, so
        // a reader can re-derive the sample with any FNV-1a implementation.
        assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a("foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn the_sample_is_not_ordered_by_url_structure() {
        // 1,000 sibling URLs under one host, sampled to 50. Lexicographic
        // order would take 0000-0049 and nothing else; the hash must not.
        let urls: Vec<String> = (0..1000)
            .map(|i| format!("https://evoevo.ai/agent/{i:04}"))
            .collect();
        let mut sorted = urls.clone();
        sorted.sort_by_key(|u| (fnv1a(u), u.clone()));
        sorted.truncate(50);
        let low = sorted
            .iter()
            .filter(|u| u.ends_with("0000") || u.rsplit('/').next().unwrap() < "0050")
            .count();
        assert!(
            low < 15,
            "the sample is clustered at the low end ({low}/50), which is what \
             sorting lexicographically would have produced"
        );
    }

    #[test]
    fn services_is_read_with_rung_4s_alias_rule() {
        let doc = br#"{"services":[{"name":"a2a","endpoint":"https://a.example/"}]}"#;
        let d = declared_endpoints(doc).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].0.as_deref(), Some("a2a"));
        assert_eq!(d.entries[0].1.as_deref(), Some("https://a.example/"));

        // The legacy alias is accepted...
        let legacy = br#"{"endpoints":[{"endpoint":"https://b.example/"}]}"#;
        let d = declared_endpoints(legacy).unwrap();
        assert_eq!(d.entries[0].1.as_deref(), Some("https://b.example/"));

        // ...but `services` wins when both are present, exactly as in rung 4.
        let both = br#"{"services":[{"endpoint":"https://a.example/"}],
                       "endpoints":[{"endpoint":"https://b.example/"}]}"#;
        let d = declared_endpoints(both).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].1.as_deref(), Some("https://a.example/"));
    }

    #[test]
    fn a_non_string_endpoint_is_read_as_absent_not_coerced() {
        let doc = br#"{"services":[{"endpoint":42},{"endpoint":{"url":"https://x/"}},{}]}"#;
        let d = declared_endpoints(doc).unwrap();
        assert_eq!(d.entries.len(), 3);
        assert!(d.entries.iter().all(|(_, e)| e.is_none()));
    }

    #[test]
    fn a_document_with_no_services_yields_nothing_to_probe() {
        assert!(declared_endpoints(br#"{"name":"x"}"#).is_none());
        assert!(declared_endpoints(br#"{"services":null}"#).is_none());
        // Present but not an array: nothing to walk.
        assert!(declared_endpoints(br#"{"services":{"a":1}}"#).is_none());
        // An empty array IS a claim — zero entries — and must be distinguished
        // from an absent field, because `checks::live` reports the two with
        // different reasons.
        let d = declared_endpoints(br#"{"services":[]}"#).unwrap();
        assert!(d.entries.is_empty());
    }

    #[test]
    fn unparseable_bytes_never_panic() {
        assert!(declared_endpoints(b"not json").is_none());
        assert!(declared_endpoints(b"").is_none());
        assert!(declared_endpoints(&[0xff, 0xfe]).is_none());
    }

    #[test]
    fn a_rung_4_that_did_not_pass_skips_rung_6_through_the_ladder() {
        let now = Utc::now();
        for status in ["fail", "error", "skipped", "", "unclaimed"] {
            let out = checks::run_ladder(vec![
                rung4_as_read(status, now),
                checks::CheckResult {
                    rung: 6,
                    name: "live",
                    status: checks::CheckStatus::Pass,
                    evidence: serde_json::json!({}),
                    checked_at: now,
                },
            ]);
            let r6 = out.iter().find(|r| r.rung == 6).unwrap();
            assert_eq!(
                r6.status,
                checks::CheckStatus::Skipped,
                "rung 4 = {status:?} must skip rung 6"
            );
            assert_eq!(r6.evidence["skipped_because_rung"], 4);
        }
    }

    #[test]
    fn a_passing_rung_4_lets_rung_6_keep_its_own_verdict() {
        let now = Utc::now();
        for status in [
            checks::CheckStatus::Pass,
            checks::CheckStatus::Fail,
            checks::CheckStatus::Unprobeable,
        ] {
            let out = checks::run_ladder(vec![
                rung4_as_read("pass", now),
                checks::CheckResult {
                    rung: 6,
                    name: "live",
                    status,
                    evidence: serde_json::json!({}),
                    checked_at: now,
                },
            ]);
            let r6 = out.iter().find(|r| r.rung == 6).unwrap();
            assert_eq!(r6.status, status);
        }
    }
}
