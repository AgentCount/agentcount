//! Rung 6 — `live`: does anything answer at the endpoints the agent declared?
//!
//! The last rung to ship, and the first that touches a third party's server
//! rather than reading a chain or a document. That is why it waited: the
//! method needed settling, and the probe's User-Agent had to carry a domain
//! that resolves and a mailbox that answers before it was decent to send a
//! single request.
//!
//! Like every other rung, this file is PURE. It judges observations the
//! sweeper collected; it does not make requests. `crates/probe` fetches,
//! `crates/sweeper` schedules, this decides.
//!
//! # Liveness is not functionality
//!
//! This rung answers one narrow question — *did something answer* — and it is
//! worth being blunt about what that excludes, because "live" is a word people
//! will read more into than it can carry:
//!
//! * A `GET` returning 404 may front a perfectly working POST-only service.
//!   This rung reports the 404 verbatim and calls it not live. It is not a
//!   claim that the agent does not work.
//! * A 200 may be a parking page, a login wall, or a load balancer's default
//!   backend. This rung does not read the body and makes no claim about what
//!   answered.
//! * Nothing here checks that the thing answering is the agent, that it speaks
//!   any particular protocol, or that it would do anything useful if asked.
//!
//! `METHODOLOGY.md` says the same in the published definition. A rung that
//! measures reachability must not be quoted as one that measures capability.
//!
//! # The three rulings this rung implements
//!
//! ## 1. Live means 2xx **or 402**
//!
//! A 402 is a payment challenge, which means something is there, listening,
//! and demanding money. That is the strongest possible evidence of liveness
//! short of a 200 — a dead host does not bill you. It passes, and is counted
//! separately as `payment_gated` so no reader has to take "live" on trust.
//!
//! **This is deliberately the opposite of rung 2's ruling on the same status**,
//! and the two are not in conflict because they ask different questions. Rung
//! 2 asks whether the registration document could be RETRIEVED; a 402 means it
//! could not, so rung 2 fails. Rung 6 asks whether anything is ALIVE at the
//! service endpoint; a 402 proves it is. Same status code, two questions, two
//! correct answers. Anyone reconciling the two counts needs that sentence.
//!
//! Every other status is `fail`, with the code recorded verbatim in evidence
//! rather than bucketed — a reader wanting to separate 404s from 503s can, and
//! this rung does not decide for them which of those is the agent's fault.
//!
//! ## 2. Only `http(s)` is probeable, and having nothing probeable is its own status
//!
//! Each declared entry is classified into one of six kinds ([`EndpointKind`]).
//! Only `http` and `https` are dialled. An agent whose every entry is a
//! CAIP-10 chain address, an email address, an empty string or a missing
//! `endpoint` field gets [`CheckStatus::Unprobeable`] — rung 5's `unclaimed`
//! reasoning, one rung over. Across the four-chain census that is 11.0% of all
//! declared entries, so this is not an edge case being tidied away.
//!
//! ## 3. Distinct URLs are probed once, and mega-hosts are sampled
//!
//! 125,705 HTTP(S) endpoints resolve to 3,399 distinct hosts, and four hosts
//! carry 59.2% of them. Probing every declared endpoint would send 26,273
//! requests to `evoevo.ai` alone to learn one fact about one server, which is
//! indistinguishable from an attack and would not be more true for the volume.
//!
//! So the sweeper deduplicates exact URLs, then applies a per-host budget of
//! distinct URLs. An agent whose URL was probed carries its observation. An
//! agent whose URL fell outside the budget carries no observation — and if it
//! has no probed endpoint at all, [`live`] returns `None` and **the agent gets
//! no rung-6 row**.
//!
//! That last point is the one to be careful about. It would be easy, and
//! wrong, to give an unprobed agent its host's sampled rate: that is
//! *inferring* a status for an agent nobody checked, and this project's whole
//! discipline is that "we did not ask" is a different claim from "we asked".
//! Absence is the honest answer, and the published rate is stated over the
//! agents actually probed.
//!
//! # Aggregating several endpoints into one verdict
//!
//! An agent may declare many services. This rung passes if **any** probeable
//! endpoint answered live, which is the same any-match rule rung 5 uses for
//! `registrations`: the question is whether the agent is reachable, and one
//! working endpoint means it is. Every endpoint's own outcome is in evidence
//! along with the counts, so a reader who wants "all endpoints live" can
//! compute it without this rung having to pick that definition for them.
//!
//! When nothing is live, the tie-break between `fail` and `error` follows rung
//! 2's line exactly, so the two rungs can never disagree about who is at
//! fault: a definite non-live ANSWER from the server (any status that is not
//! 2xx or 402), or an `ssrf_blocked` rejection — which means no third party
//! could have reached it either — is the agent's fact, so `fail`. A timeout, a
//! TLS failure, a robots.txt we could not read or that disallowed us: those
//! are OUR limitations, so `error`, never `fail`. Getting this backwards
//! publishes a false accusation about a real project.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

use crate::model::{CheckResult, CheckStatus};

/// What a declared `services[].endpoint` turned out to be.
///
/// The kinds a reader is offered are the ones the ruling named — `https`,
/// `http`, `caip10`, `email`, `missing`, `other` — and the raw string is
/// always carried in evidence beside the kind, so a shape folded into `Other`
/// (an `ipfs://` URI, an empty string, a bare hostname) is still countable
/// afterwards without this enum having to grow a variant per curiosity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    Https,
    Http,
    /// `eip155:1:0x…` and friends — a chain address, not a network endpoint.
    Caip10,
    /// `mailto:…`, or a bare `name@host` that is plainly an address.
    Email,
    /// The entry declared no `endpoint` field, or declared it as `null`.
    Missing,
    /// Anything else: an empty string, an `ipfs://` URI, a bare hostname, a
    /// sentence. Not dialled, and never guessed at.
    Other,
}

impl EndpointKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EndpointKind::Https => "https",
            EndpointKind::Http => "http",
            EndpointKind::Caip10 => "caip10",
            EndpointKind::Email => "email",
            EndpointKind::Missing => "missing",
            EndpointKind::Other => "other",
        }
    }

    /// Whether the prober will dial this. Only `http` and `https` — see the
    /// module doc's ruling 2.
    pub fn is_probeable(&self) -> bool {
        matches!(self, EndpointKind::Https | EndpointKind::Http)
    }
}

/// Classify one declared endpoint string.
///
/// Pure and total: every input produces a kind, and nothing is rejected. The
/// order of the checks matters — `mailto:` is a scheme and would otherwise
/// fall through to `Other`, and a CAIP-10 identifier looks enough like a
/// scheme-prefixed URI (`eip155:1:0x…`) that it has to be recognised before
/// any generic scheme handling.
pub fn classify_endpoint(raw: Option<&str>) -> EndpointKind {
    let Some(raw) = raw else {
        return EndpointKind::Missing;
    };
    let s = raw.trim();
    if s.is_empty() {
        // Not `Missing`: the agent wrote the field and left it blank, which is
        // a different act from omitting it. Both are unprobeable; only one of
        // them is a typo.
        return EndpointKind::Other;
    }

    let lower = s.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return EndpointKind::Https;
    }
    if lower.starts_with("http://") {
        return EndpointKind::Http;
    }
    if lower.starts_with("mailto:") {
        return EndpointKind::Email;
    }

    // CAIP-10: `<namespace>:<reference>:<address>`, e.g. `eip155:1:0xabc…`.
    // Matched structurally rather than against a namespace allowlist, because
    // the point is to recognise "this is an on-chain identifier, not a URL"
    // and a namespace we have not heard of is still not a URL.
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 3
        && !parts[0].is_empty()
        && !parts[1].is_empty()
        && !parts[2].is_empty()
        && parts
            .iter()
            .all(|p| !p.contains('/') && !p.contains(' ') && !p.contains('@'))
    {
        return EndpointKind::Caip10;
    }

    // A bare email address — `support@example.com`. Required to have exactly
    // one `@`, something either side, a dot in the domain, and no slash or
    // space, so a URL missing its scheme is not swept up as an address.
    if !s.contains('/') && !s.contains(' ') {
        let at: Vec<&str> = s.split('@').collect();
        if at.len() == 2 && !at[0].is_empty() && at[1].contains('.') && !at[1].starts_with('.') {
            return EndpointKind::Email;
        }
    }

    EndpointKind::Other
}

/// What the prober saw at one URL. Assembled by the sweeper from a
/// `probe::FetchOutcome`; this crate never learns how the bytes were obtained.
#[derive(Debug, Clone, Default)]
pub struct EndpointObservation {
    pub http_status: Option<u16>,
    /// Plain text describing why no usable response came back — `timeout`,
    /// `tls`, `robots_disallowed`, `robots_unavailable: …`, or an
    /// `ssrf_blocked: …` netguard rejection. Never a verdict; this rung is the
    /// only place that turns it into one, and it uses rung 2's exact rule.
    pub error: Option<String>,
    pub elapsed_ms: Option<u32>,
    /// Where the request ended up after redirects, when it got that far.
    pub final_url: Option<String>,
}

/// One entry of the document's `services` array, reduced to what this rung
/// needs.
#[derive(Debug, Clone)]
pub struct ServiceEndpoint {
    /// Position in the declared array, so evidence points at the exact entry
    /// rather than at a URL that may repeat.
    pub index: usize,
    /// The entry's `name`, when it declared one. Carried through untouched.
    pub name: Option<String>,
    /// The endpoint exactly as written. `None` when the field was absent.
    pub declared: Option<String>,
    /// The URL actually dialled, after whatever normalisation the sweeper did
    /// on the way to deduplicating. `None` when nothing was dialled.
    pub probed_url: Option<String>,
    /// The observation, when this endpoint was probed. `None` both for an
    /// unprobeable kind and for a probeable URL that fell outside its host's
    /// sampling budget — [`ServiceEndpoint::kind`] distinguishes the two.
    pub observed: Option<EndpointObservation>,
}

impl ServiceEndpoint {
    pub fn kind(&self) -> EndpointKind {
        classify_endpoint(self.declared.as_deref())
    }
}

pub struct LiveInput {
    /// Every entry of the document's `services` (or legacy `endpoints`) array,
    /// in declared order. Empty when the document declared no services at all
    /// — which is `Unprobeable`, the same as declaring only unprobeable ones.
    pub endpoints: Vec<ServiceEndpoint>,
    /// True when at least one of this agent's probeable URLs was left
    /// undialled because its host's budget was already spent. Recorded in
    /// evidence so a `pass` from one endpoint never hides that others went
    /// unasked.
    pub host_budget_reached: bool,
}

/// Judge one agent's declared endpoints.
///
/// Returns `None` when the agent HAS probeable endpoints but none of them was
/// probed — the sampling case from the module doc's ruling 3. `None` means the
/// caller must write no rung-6 row at all, because absence is this project's
/// only honest way to say "not asked". Every other case returns a result.
pub fn live(input: &LiveInput, now: DateTime<Utc>) -> Option<CheckResult> {
    let mut per_endpoint: Vec<Value> = Vec::with_capacity(input.endpoints.len());
    let mut probeable = 0usize;
    let mut probed = 0usize;
    let mut live_count = 0usize;
    let mut payment_gated = 0usize;
    // "The server answered, and the answer was not live" — the agent's fact.
    let mut answered_not_live = 0usize;
    // "We could not complete the request" — ours.
    let mut our_failures = 0usize;

    for e in &input.endpoints {
        let kind = e.kind();
        let mut row = Map::new();
        row.insert("index".into(), json!(e.index));
        row.insert("kind".into(), json!(kind.as_str()));
        // Always the raw string, so a shape folded into `other` stays
        // countable later without this rung growing a variant for it.
        row.insert("declared".into(), json!(e.declared));
        if let Some(n) = &e.name {
            row.insert("name".into(), json!(n));
        }

        if !kind.is_probeable() {
            row.insert("probed".into(), json!(false));
            per_endpoint.push(Value::Object(row));
            continue;
        }
        probeable += 1;

        let Some(obs) = &e.observed else {
            // Probeable, not probed: the host's budget was spent before this
            // URL came up. Distinguished from an unprobeable kind by `kind`
            // still reading `https`/`http`.
            row.insert("probed".into(), json!(false));
            row.insert("not_probed_because".into(), json!("host_sampling_budget"));
            per_endpoint.push(Value::Object(row));
            continue;
        };

        probed += 1;
        row.insert("probed".into(), json!(true));
        if let Some(u) = &e.probed_url {
            row.insert("probed_url".into(), json!(u));
        }
        if let Some(u) = &obs.final_url {
            row.insert("final_url".into(), json!(u));
        }
        if let Some(s) = obs.http_status {
            row.insert("http_status".into(), json!(s));
        }
        if let Some(ms) = obs.elapsed_ms {
            row.insert("elapsed_ms".into(), json!(ms));
        }

        // The one place a verdict is derived from an observation. Rung 2's
        // rule, with 2xx widened to include 402 — see the module doc.
        let outcome = if let Some(err) = &obs.error {
            row.insert("error".into(), json!(err));
            if err.starts_with("ssrf_blocked: ") {
                // No third party could have reached this either, so it is a
                // fact about what the agent published — not a limit of ours.
                answered_not_live += 1;
                "not_live"
            } else {
                our_failures += 1;
                "our_error"
            }
        } else if let Some(code) = obs.http_status {
            if (200..300).contains(&code) {
                live_count += 1;
                "live"
            } else if code == 402 {
                live_count += 1;
                payment_gated += 1;
                row.insert("payment_gated".into(), json!(true));
                "live"
            } else {
                answered_not_live += 1;
                "not_live"
            }
        } else {
            // Neither an error nor a status. The prober owes us one or the
            // other for anything it dialled; treat the gap as OUR bug rather
            // than let it become a silent pass or a false accusation.
            row.insert("error".into(), json!("no_response"));
            our_failures += 1;
            "our_error"
        };
        row.insert("outcome".into(), json!(outcome));
        per_endpoint.push(Value::Object(row));
    }

    // Probeable endpoints exist but none was reached: not a status, an
    // absence. See ruling 3 in the module doc.
    if probeable > 0 && probed == 0 {
        return None;
    }

    let status = if probeable == 0 {
        CheckStatus::Unprobeable
    } else if live_count > 0 {
        CheckStatus::Pass
    } else if answered_not_live > 0 {
        CheckStatus::Fail
    } else {
        // Everything we tried failed on our side.
        CheckStatus::Error
    };

    let mut evidence = Map::new();
    evidence.insert("endpoints_declared".into(), json!(input.endpoints.len()));
    evidence.insert("endpoints_probeable".into(), json!(probeable));
    evidence.insert("endpoints_probed".into(), json!(probed));
    evidence.insert("endpoints_live".into(), json!(live_count));
    evidence.insert("endpoints_payment_gated".into(), json!(payment_gated));
    evidence.insert(
        "endpoints_answered_not_live".into(),
        json!(answered_not_live),
    );
    evidence.insert("endpoints_our_error".into(), json!(our_failures));
    // A `pass` from one endpoint must never hide that others went unasked.
    if input.host_budget_reached {
        evidence.insert("host_sampling_budget_reached".into(), json!(true));
    }
    if status == CheckStatus::Unprobeable {
        // Name the reason in the row itself, so a count of `unprobeable` can
        // be broken down without re-reading every endpoint array.
        let kinds: Vec<&str> = input.endpoints.iter().map(|e| e.kind().as_str()).collect();
        evidence.insert(
            "reason".into(),
            json!(if input.endpoints.is_empty() {
                "no_services_declared"
            } else {
                "no_probeable_endpoint"
            }),
        );
        evidence.insert("kinds".into(), json!(kinds));
    }
    evidence.insert("endpoints".into(), json!(per_endpoint));

    Some(CheckResult {
        rung: 6,
        name: "live",
        status,
        evidence: Value::Object(evidence),
        checked_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    fn t() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    /// One probed https endpoint. Most tests mutate a field off this.
    fn ep(url: &str, obs: Option<EndpointObservation>) -> ServiceEndpoint {
        ServiceEndpoint {
            index: 0,
            name: Some("agentCard".into()),
            declared: Some(url.into()),
            probed_url: Some(url.into()),
            observed: obs,
        }
    }

    fn ok(status: u16) -> Option<EndpointObservation> {
        Some(EndpointObservation {
            http_status: Some(status),
            error: None,
            elapsed_ms: Some(140),
            final_url: None,
        })
    }

    fn err(reason: &str) -> Option<EndpointObservation> {
        Some(EndpointObservation {
            http_status: None,
            error: Some(reason.into()),
            elapsed_ms: None,
            final_url: None,
        })
    }

    fn one(e: ServiceEndpoint) -> LiveInput {
        LiveInput {
            endpoints: vec![e],
            host_budget_reached: false,
        }
    }

    // ── classification (ruling 2) ────────────────────────────────────────

    #[test]
    fn schemes_classify_and_only_http_is_probeable() {
        for (raw, want, probeable) in [
            ("https://example.com/a2a", EndpointKind::Https, true),
            ("HTTPS://EXAMPLE.COM/", EndpointKind::Https, true),
            ("http://example.com/a2a", EndpointKind::Http, true),
            ("mailto:ops@example.com", EndpointKind::Email, false),
            ("support@example.com", EndpointKind::Email, false),
            (
                "eip155:1:0xabc0000000000000000000000000000000000000",
                EndpointKind::Caip10,
                false,
            ),
            ("solana:mainnet:9xQeWv", EndpointKind::Caip10, false),
            ("ipfs://bafybeigdyrzt", EndpointKind::Other, false),
            ("", EndpointKind::Other, false),
            ("   ", EndpointKind::Other, false),
            ("example.com", EndpointKind::Other, false),
            ("coming soon", EndpointKind::Other, false),
        ] {
            let got = classify_endpoint(Some(raw));
            assert_eq!(got, want, "{raw:?} classified as {}", got.as_str());
            assert_eq!(got.is_probeable(), probeable, "{raw:?} probeable");
        }
        assert_eq!(classify_endpoint(None), EndpointKind::Missing);
    }

    #[test]
    fn an_empty_endpoint_is_other_not_missing() {
        // The agent wrote the field and left it blank. Both are unprobeable;
        // only one of them is a typo, and the census counts them apart.
        assert_eq!(classify_endpoint(Some("")), EndpointKind::Other);
        assert_eq!(classify_endpoint(None), EndpointKind::Missing);
    }

    #[test]
    fn a_url_missing_its_scheme_is_not_mistaken_for_an_email() {
        assert_eq!(
            classify_endpoint(Some("api.example.com/v1")),
            EndpointKind::Other
        );
        assert_eq!(
            classify_endpoint(Some("user@host/path")),
            EndpointKind::Other
        );
    }

    // ── ruling 1: live is 2xx or 402 ─────────────────────────────────────

    #[test]
    fn a_2xx_is_live() {
        for code in [200u16, 201, 204, 299] {
            let r = live(&one(ep("https://example.com/", ok(code))), t()).unwrap();
            assert_eq!(r.status, CheckStatus::Pass, "{code} should be live");
            assert_eq!(r.evidence["endpoints_live"], 1);
        }
    }

    #[test]
    fn a_402_is_live_and_counted_as_payment_gated() {
        // A dead host does not bill you. This is the ruling that diverges from
        // rung 2, where the same status fails — see the module doc.
        let r = live(&one(ep("https://example.com/", ok(402))), t()).unwrap();
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["endpoints_payment_gated"], 1);
        assert_eq!(r.evidence["endpoints"][0]["payment_gated"], true);
        assert_eq!(r.evidence["endpoints"][0]["http_status"], 402);
    }

    #[test]
    fn every_other_status_fails_with_the_code_recorded_verbatim() {
        for code in [301u16, 400, 401, 403, 404, 429, 500, 503] {
            let r = live(&one(ep("https://example.com/", ok(code))), t()).unwrap();
            assert_eq!(r.status, CheckStatus::Fail, "{code} should not be live");
            assert_eq!(
                r.evidence["endpoints"][0]["http_status"], code,
                "the code itself must survive into evidence, not a bucket"
            );
            assert_eq!(r.evidence["endpoints"][0]["outcome"], "not_live");
        }
    }

    #[test]
    fn a_404_is_a_fail_and_the_methodology_says_why_that_is_not_a_verdict_on_the_service() {
        // Pinned as a fixture because it is the case most likely to be
        // misquoted: a GET 404 may front a POST-only service.
        let r = live(&one(ep("https://example.com/rpc", ok(404))), t()).unwrap();
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["endpoints_answered_not_live"], 1);
    }

    // ── our failures are never the agent's fail ──────────────────────────

    #[test]
    fn a_timeout_or_tls_or_robots_failure_is_our_error_never_a_fail() {
        for reason in [
            "timeout",
            "tls",
            "dns",
            "robots_disallowed",
            "robots_unavailable: robots.txt returned HTTP 503",
        ] {
            let r = live(&one(ep("https://example.com/", err(reason))), t()).unwrap();
            assert_eq!(r.status, CheckStatus::Error, "{reason} should be Error");
            assert_ne!(r.status, CheckStatus::Fail);
            assert_eq!(r.evidence["endpoints_our_error"], 1);
        }
    }

    #[test]
    fn an_ssrf_block_is_the_agents_fact_and_fails_exactly_as_in_rung_2() {
        // An endpoint that does not resolve, or resolves only to a private
        // address, could not have been reached by anyone. Rung 2 already
        // treats this as the agent's fact; the two rungs must not disagree
        // about who is at fault for the same string.
        for reason in [
            "ssrf_blocked: dns resolution failed: no record found",
            "ssrf_blocked: resolves to a non-public address",
        ] {
            let r = live(&one(ep("https://example.com/", err(reason))), t()).unwrap();
            assert_eq!(r.status, CheckStatus::Fail, "{reason} should be Fail");
            assert_eq!(r.evidence["endpoints_answered_not_live"], 1);
            assert_eq!(r.evidence["endpoints_our_error"], 0);
        }
    }

    #[test]
    fn a_probe_that_returned_neither_status_nor_error_is_our_bug_not_a_pass() {
        let r = live(
            &one(ep(
                "https://example.com/",
                Some(EndpointObservation::default()),
            )),
            t(),
        )
        .unwrap();
        assert_eq!(r.status, CheckStatus::Error);
        assert_eq!(r.evidence["endpoints"][0]["error"], "no_response");
    }

    // ── ruling 2: unprobeable ────────────────────────────────────────────

    #[test]
    fn no_services_at_all_is_unprobeable() {
        let r = live(
            &LiveInput {
                endpoints: vec![],
                host_budget_reached: false,
            },
            t(),
        )
        .unwrap();
        assert_eq!(r.status, CheckStatus::Unprobeable);
        assert_eq!(r.evidence["reason"], "no_services_declared");
        assert_eq!(r.evidence["endpoints_probeable"], 0);
    }

    #[test]
    fn only_unprobeable_kinds_is_unprobeable_and_names_them() {
        let input = LiveInput {
            endpoints: vec![
                ep("eip155:1:0xabc0000000000000000000000000000000000000", None),
                ep("ops@example.com", None),
                ServiceEndpoint {
                    index: 2,
                    name: None,
                    declared: None,
                    probed_url: None,
                    observed: None,
                },
            ],
            host_budget_reached: false,
        };
        let r = live(&input, t()).unwrap();
        assert_eq!(r.status, CheckStatus::Unprobeable);
        assert_eq!(r.evidence["reason"], "no_probeable_endpoint");
        assert_eq!(r.evidence["kinds"], json!(["caip10", "email", "missing"]));
        // Not `Unclaimed`, and not `Fail`: the agent declared something, it is
        // simply not something a prober can dial.
        assert_ne!(r.status, CheckStatus::Unclaimed);
        assert_ne!(r.status, CheckStatus::Fail);
    }

    #[test]
    fn unprobeable_is_its_own_word_on_the_wire() {
        assert_eq!(CheckStatus::Unprobeable.as_str(), "unprobeable");
        assert_ne!(CheckStatus::Unprobeable, CheckStatus::Unclaimed);
    }

    // ── ruling 3: sampling ───────────────────────────────────────────────

    #[test]
    fn a_probeable_agent_with_nothing_probed_gets_no_row_at_all() {
        // The sampling case. Inferring this agent's status from its host's
        // sampled rate would be a status for an agent nobody checked.
        let input = LiveInput {
            endpoints: vec![ep("https://evoevo.ai/a", None)],
            host_budget_reached: true,
        };
        assert!(
            live(&input, t()).is_none(),
            "absence is the only honest answer for an agent that was not probed"
        );
    }

    #[test]
    fn a_partly_sampled_agent_is_judged_on_what_was_probed_and_says_so() {
        let input = LiveInput {
            endpoints: vec![
                ServiceEndpoint {
                    index: 0,
                    name: None,
                    declared: Some("https://a.example/".into()),
                    probed_url: Some("https://a.example/".into()),
                    observed: ok(200),
                },
                ServiceEndpoint {
                    index: 1,
                    name: None,
                    declared: Some("https://evoevo.ai/x".into()),
                    probed_url: None,
                    observed: None,
                },
            ],
            host_budget_reached: true,
        };
        let r = live(&input, t()).unwrap();
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["endpoints_probeable"], 2);
        assert_eq!(r.evidence["endpoints_probed"], 1);
        // A pass from one endpoint must not hide that another went unasked.
        assert_eq!(r.evidence["host_sampling_budget_reached"], true);
        assert_eq!(
            r.evidence["endpoints"][1]["not_probed_because"],
            "host_sampling_budget"
        );
        assert_eq!(
            r.evidence["endpoints"][1]["kind"], "https",
            "still probeable — the kind is what separates this from unprobeable"
        );
    }

    // ── aggregation ──────────────────────────────────────────────────────

    #[test]
    fn any_live_endpoint_passes_the_agent() {
        let input = LiveInput {
            endpoints: vec![
                ServiceEndpoint {
                    index: 0,
                    name: None,
                    declared: Some("https://a.example/".into()),
                    probed_url: Some("https://a.example/".into()),
                    observed: ok(500),
                },
                ServiceEndpoint {
                    index: 1,
                    name: None,
                    declared: Some("https://b.example/".into()),
                    probed_url: Some("https://b.example/".into()),
                    observed: ok(200),
                },
            ],
            host_budget_reached: false,
        };
        let r = live(&input, t()).unwrap();
        assert_eq!(r.status, CheckStatus::Pass);
        // Both outcomes survive, so "all endpoints live" stays computable by a
        // reader who wants that definition instead.
        assert_eq!(r.evidence["endpoints_live"], 1);
        assert_eq!(r.evidence["endpoints_answered_not_live"], 1);
    }

    #[test]
    fn our_error_alongside_a_definite_refusal_still_fails() {
        // One endpoint answered 404 — that is the agent's fact and stands on
        // its own, regardless of the other having timed out on us.
        let input = LiveInput {
            endpoints: vec![
                ServiceEndpoint {
                    index: 0,
                    name: None,
                    declared: Some("https://a.example/".into()),
                    probed_url: Some("https://a.example/".into()),
                    observed: err("timeout"),
                },
                ServiceEndpoint {
                    index: 1,
                    name: None,
                    declared: Some("https://b.example/".into()),
                    probed_url: Some("https://b.example/".into()),
                    observed: ok(404),
                },
            ],
            host_budget_reached: false,
        };
        let r = live(&input, t()).unwrap();
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.evidence["endpoints_our_error"], 1);
        assert_eq!(r.evidence["endpoints_answered_not_live"], 1);
    }

    #[test]
    fn only_our_errors_is_error_and_never_blames_the_agent() {
        let input = LiveInput {
            endpoints: vec![
                ServiceEndpoint {
                    index: 0,
                    name: None,
                    declared: Some("https://a.example/".into()),
                    probed_url: Some("https://a.example/".into()),
                    observed: err("timeout"),
                },
                ServiceEndpoint {
                    index: 1,
                    name: None,
                    declared: Some("https://b.example/".into()),
                    probed_url: Some("https://b.example/".into()),
                    observed: err("tls"),
                },
            ],
            host_budget_reached: false,
        };
        let r = live(&input, t()).unwrap();
        assert_eq!(r.status, CheckStatus::Error);
    }

    #[test]
    fn unprobeable_entries_alongside_a_live_one_do_not_change_the_verdict() {
        let input = LiveInput {
            endpoints: vec![
                ep("eip155:1:0xabc0000000000000000000000000000000000000", None),
                ServiceEndpoint {
                    index: 1,
                    name: None,
                    declared: Some("https://b.example/".into()),
                    probed_url: Some("https://b.example/".into()),
                    observed: ok(200),
                },
            ],
            host_budget_reached: false,
        };
        let r = live(&input, t()).unwrap();
        assert_eq!(r.status, CheckStatus::Pass);
        assert_eq!(r.evidence["endpoints_declared"], 2);
        assert_eq!(r.evidence["endpoints_probeable"], 1);
        assert_eq!(r.evidence["endpoints"][0]["probed"], false);
        assert!(
            r.evidence["endpoints"][0]
                .get("not_probed_because")
                .is_none()
        );
    }

    // ── evidence shape ───────────────────────────────────────────────────

    #[test]
    fn every_declared_endpoint_appears_in_evidence_with_its_raw_string() {
        // The raw string is what lets a shape folded into `other` — an
        // `ipfs://` URI, an empty string — stay countable afterwards.
        let input = LiveInput {
            endpoints: vec![
                ep("ipfs://bafybeigdyrzt", None),
                ep("", None),
                ServiceEndpoint {
                    index: 2,
                    name: None,
                    declared: None,
                    probed_url: None,
                    observed: None,
                },
            ],
            host_budget_reached: false,
        };
        let r = live(&input, t()).unwrap();
        let eps = r.evidence["endpoints"].as_array().unwrap();
        assert_eq!(eps.len(), 3);
        assert_eq!(eps[0]["declared"], "ipfs://bafybeigdyrzt");
        assert_eq!(eps[1]["declared"], "");
        assert_eq!(eps[2]["declared"], Value::Null);
        assert!(eps.iter().all(|e| e["kind"].is_string()));
    }

    #[test]
    fn the_result_is_always_rung_6_named_live() {
        let r = live(&one(ep("https://example.com/", ok(200))), t()).unwrap();
        assert_eq!(r.rung, 6);
        assert_eq!(r.name, "live");
        assert_eq!(r.checked_at, t());
    }
}
