//! robots.txt: fetched once per host, cached for the process, and consulted
//! before we fetch anything else from that host.
//!
//! The outcome mapping is the point of this module, because it decides
//! whether an agent gets blamed for OUR problem:
//!
//! * 2xx  → parse it and apply the rules.
//! * 4xx (including 404) → no robots.txt means no restriction; proceed.
//! * 5xx, timeout, or connection failure → we could not establish
//!   permission, so we act as if disallowed for this run — but tagged
//!   [`RobotsDecision::Unavailable`] so the caller records it as OUR error,
//!   never the agent's failure.
//!
//! **Redirects.** A redirect on `/robots.txt` is completely ordinary —
//! `http`→`https`, `www`→apex — and RFC 9309 §2.3.1.2 requires following at
//! least five of them before giving up. We follow up to
//! [`MAX_ROBOTS_REDIRECTS`] hops, re-running the netguard's SSRF check on
//! every hop exactly as the main document fetch does (a redirect is
//! attacker-controlled input, robots.txt's redirects included), then apply
//! the mapping above to whatever response is left standing. A chain that
//! loops back on itself and one that simply runs past the cap look
//! identical from here — both exhaust the hop budget — so both land on
//! [`RobotsRules::Unavailable`], never a hang or a crash.
//!
//! Parsing itself is deliberately small: we honour `Disallow` for our exact
//! product token and for `*` (unioned, not "most specific wins" — the brief
//! asks for both to be honoured, not for full RFC 9309 precedence), and we do
//! not implement `Allow:` overrides. Good enough for politeness; not a
//! general-purpose robots.txt parser.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, OnceCell};

use crate::fetch::{Prober, SendError};

/// Redirect hops followed while fetching `robots.txt` before giving up.
/// RFC 9309 §2.3.1.2: "clients SHOULD follow at least five consecutive
/// redirects". Independent of [`crate::fetch::MAX_REDIRECTS`], which bounds
/// redirects on the agent's actual registration document — the two are
/// governed by different rules for different reasons.
pub(crate) const MAX_ROBOTS_REDIRECTS: u8 = 5;

/// Per-path answer for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RobotsDecision {
    Allowed,
    Disallowed,
    /// Could not determine permission (5xx / timeout / connection failure).
    /// Treated as disallowed for this run, but distinguishable from an
    /// explicit `Disallow` — the reason is OURS, not the agent's.
    Unavailable(String),
}

/// What we learned about one origin's robots.txt — cached, independent of
/// any one path.
#[derive(Debug, Clone)]
enum RobotsRules {
    /// No robots.txt (404/4xx), or a robots.txt with no rules that apply to us.
    Open,
    /// Disallow prefixes that apply to our product token or `*`.
    Restricted(Vec<String>),
    Unavailable(String),
}

fn apply_rules(rules: &RobotsRules, path: &str) -> RobotsDecision {
    match rules {
        RobotsRules::Open => RobotsDecision::Allowed,
        RobotsRules::Unavailable(reason) => RobotsDecision::Unavailable(reason.clone()),
        RobotsRules::Restricted(prefixes) => {
            if prefixes
                .iter()
                .any(|p| !p.is_empty() && path.starts_with(p.as_str()))
            {
                RobotsDecision::Disallowed
            } else {
                RobotsDecision::Allowed
            }
        }
    }
}

/// Per-host cache, keyed by origin (`scheme://host[:port]`). One entry per
/// origin for the life of the `Prober`; concurrent first-lookups for the same
/// origin share one fetch via `OnceCell` rather than racing.
pub(crate) struct RobotsCache {
    entries: Mutex<HashMap<String, Arc<OnceCell<RobotsRules>>>>,
}

impl RobotsCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}

impl Prober {
    /// Is `path` on `origin`'s host allowed? Fetches and parses robots.txt
    /// the first time this origin is seen (through the same per-host/global
    /// caps and body cap as any other request — a robots.txt is still a
    /// response from a server we don't control); every later call for the
    /// same origin reuses the cached decision. `validate_hops` mirrors
    /// [`crate::fetch::Prober::fetch_http`]'s parameter of the same name —
    /// `false` only in this crate's own mechanics tests, which run against a
    /// loopback `wiremock` server the netguard would otherwise (correctly)
    /// refuse; the production entry point always passes `true`.
    pub(crate) async fn check_robots(
        &self,
        origin: &url::Url,
        path: &str,
        validate_hops: bool,
    ) -> RobotsDecision {
        let key = origin.origin().ascii_serialization();
        let cell = {
            let mut entries = self.robots.entries.lock().await;
            entries
                .entry(key)
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone()
        };
        let rules = cell
            .get_or_init(|| self.fetch_robots_rules(origin, validate_hops))
            .await;
        apply_rules(rules, path)
    }

    /// Fetch and parse `robots.txt`, following redirects per RFC 9309
    /// §2.3.1.2 (see the module doc for the redirect/loop handling).
    async fn fetch_robots_rules(&self, origin: &url::Url, validate_hops: bool) -> RobotsRules {
        let mut robots_url = origin.clone();
        robots_url.set_path("/robots.txt");
        robots_url.set_query(None);

        let mut current = robots_url;

        for hop in 0..=MAX_ROBOTS_REDIRECTS {
            if validate_hops && let Err(reason) = self.validate_hop(&current).await {
                return RobotsRules::Unavailable(format!("ssrf_blocked: {reason}"));
            }

            let resp = match self.guarded_send(&current).await {
                Ok(r) => r,
                Err(SendError::Timeout) => {
                    return RobotsRules::Unavailable("timeout fetching robots.txt".into());
                }
                Err(SendError::Connection(e)) => {
                    return RobotsRules::Unavailable(format!(
                        "connection failed fetching robots.txt: {e}"
                    ));
                }
            };

            let is_redirect = (300..400).contains(&resp.status);
            if is_redirect && hop < MAX_ROBOTS_REDIRECTS {
                match resp.location.as_deref().map(|loc| current.join(loc)) {
                    Some(Ok(next)) => {
                        current = next;
                        continue;
                    }
                    Some(Err(_)) => {
                        return RobotsRules::Unavailable(
                            "robots.txt redirected to an unusable location".into(),
                        );
                    }
                    None => {
                        // A 3xx with no Location header: nothing to follow,
                        // fall through and treat the redirect status itself
                        // as the terminal response below.
                    }
                }
            }

            return if (200..300).contains(&resp.status) {
                match String::from_utf8(resp.body) {
                    Ok(text) => RobotsRules::Restricted(parse_disallow(&text)),
                    Err(_) => {
                        // A non-UTF-8 robots.txt is unusable, not a
                        // permission grant or denial we can trust — treat
                        // like any other "could not establish permission"
                        // case.
                        RobotsRules::Unavailable("robots.txt was not valid UTF-8".into())
                    }
                }
            } else if (400..500).contains(&resp.status) {
                RobotsRules::Open
            } else if is_redirect {
                // Either the chain ran past MAX_ROBOTS_REDIRECTS or it
                // looped back on itself — both exhaust the same hop budget,
                // so both are just "robots.txt was unavailable", not a
                // crash and not a permission grant.
                RobotsRules::Unavailable(format!(
                    "robots.txt redirected more than {MAX_ROBOTS_REDIRECTS} times"
                ))
            } else {
                RobotsRules::Unavailable(format!("robots.txt returned HTTP {}", resp.status))
            };
        }
        unreachable!("the loop above always returns before exhausting 0..=MAX_ROBOTS_REDIRECTS")
    }
}

/// Extract the `Disallow` prefixes that apply to our product token or `*`,
/// from a raw robots.txt body. Groups are the standard "consecutive
/// `User-agent:` lines, then directives until the next `User-agent:` line"
/// shape; `#` starts a comment.
fn parse_disallow(body: &str) -> Vec<String> {
    let mut disallows = Vec::new();
    let mut applies_to_us = false;
    let mut group_has_started = false;

    for raw_line in body.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();

        match key.as_str() {
            "user-agent" => {
                if group_has_started {
                    // A new User-agent line after directives starts a fresh group.
                    applies_to_us = false;
                    group_has_started = false;
                }
                if value.eq_ignore_ascii_case(crate::PRODUCT_TOKEN) || value == "*" {
                    applies_to_us = true;
                }
            }
            "disallow" => {
                group_has_started = true;
                if applies_to_us && !value.is_empty() {
                    disallows.push(value.to_string());
                }
            }
            _ => group_has_started = true,
        }
    }
    disallows
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn origin_of(server: &MockServer) -> url::Url {
        url::Url::parse(&server.uri()).unwrap()
    }

    fn test_prober() -> Prober {
        Prober::new_for_test(Duration::from_secs(2), Duration::from_secs(2))
    }

    #[tokio::test]
    async fn allowed_path_proceeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("User-agent: *\nDisallow: /private/\n"),
            )
            .mount(&server)
            .await;

        let prober = test_prober();
        let origin = origin_of(&server).await;
        assert_eq!(
            prober
                .check_robots(&origin, "/public/card.json", false)
                .await,
            RobotsDecision::Allowed
        );
    }

    #[tokio::test]
    async fn disallowed_path_is_blocked() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("User-agent: *\nDisallow: /private/\n"),
            )
            .mount(&server)
            .await;

        let prober = test_prober();
        let origin = origin_of(&server).await;
        assert_eq!(
            prober
                .check_robots(&origin, "/private/card.json", false)
                .await,
            RobotsDecision::Disallowed
        );
    }

    #[tokio::test]
    async fn missing_robots_txt_is_allowed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let prober = test_prober();
        let origin = origin_of(&server).await;
        assert_eq!(
            prober.check_robots(&origin, "/anything", false).await,
            RobotsDecision::Allowed
        );
    }

    #[tokio::test]
    async fn a_5xx_robots_txt_is_disallowed_with_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let prober = test_prober();
        let origin = origin_of(&server).await;
        match prober.check_robots(&origin, "/anything", false).await {
            RobotsDecision::Unavailable(_) => {}
            other => panic!("expected Unavailable (disallowed-with-error), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_single_redirect_on_robots_txt_is_followed_and_disallow_is_honoured() {
        // The ordinary case this defect was about: robots.txt sits behind a
        // 301 (http->https, www->apex, whatever) and the Disallow it serves
        // after the redirect must still be honoured.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(301).insert_header("Location", "/robots-final.txt"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/robots-final.txt"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("User-agent: *\nDisallow: /private/\n"),
            )
            .mount(&server)
            .await;

        let prober = test_prober();
        let origin = origin_of(&server).await;
        assert_eq!(
            prober
                .check_robots(&origin, "/private/card.json", false)
                .await,
            RobotsDecision::Disallowed
        );
    }

    #[tokio::test]
    async fn an_allowed_path_after_a_robots_txt_redirect_is_fetched() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(301).insert_header("Location", "/robots-final.txt"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/robots-final.txt"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("User-agent: *\nDisallow: /private/\n"),
            )
            .mount(&server)
            .await;

        let prober = test_prober();
        let origin = origin_of(&server).await;
        assert_eq!(
            prober
                .check_robots(&origin, "/public/card.json", false)
                .await,
            RobotsDecision::Allowed
        );
    }

    #[tokio::test]
    async fn a_robots_txt_redirect_chain_of_five_is_followed() {
        // RFC 9309 §2.3.1.2: clients SHOULD follow at least five consecutive
        // redirects.
        let server = MockServer::start().await;
        let hops = [
            ("/robots.txt", "/r1"),
            ("/r1", "/r2"),
            ("/r2", "/r3"),
            ("/r3", "/r4"),
            ("/r4", "/r5"),
        ];
        for (from, to) in hops {
            Mock::given(method("GET"))
                .and(path(from))
                .respond_with(ResponseTemplate::new(302).insert_header("Location", to))
                .mount(&server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/r5"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("User-agent: *\nDisallow: /private/\n"),
            )
            .mount(&server)
            .await;

        let prober = test_prober();
        let origin = origin_of(&server).await;
        assert_eq!(
            prober
                .check_robots(&origin, "/private/card.json", false)
                .await,
            RobotsDecision::Disallowed,
            "a chain of exactly 5 redirects must be followed to the end"
        );
    }

    #[tokio::test]
    async fn a_robots_txt_redirect_chain_of_six_is_unavailable() {
        let server = MockServer::start().await;
        let hops = [
            ("/robots.txt", "/r1"),
            ("/r1", "/r2"),
            ("/r2", "/r3"),
            ("/r3", "/r4"),
            ("/r4", "/r5"),
            ("/r5", "/r6"),
        ];
        for (from, to) in hops {
            Mock::given(method("GET"))
                .and(path(from))
                .respond_with(ResponseTemplate::new(302).insert_header("Location", to))
                .mount(&server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/r6"))
            .respond_with(ResponseTemplate::new(200).set_body_string("User-agent: *\n"))
            .mount(&server)
            .await;

        let prober = test_prober();
        let origin = origin_of(&server).await;
        match prober.check_robots(&origin, "/anything", false).await {
            RobotsDecision::Unavailable(_) => {}
            other => panic!(
                "a chain of 6 redirects exceeds the 5-hop cap; expected Unavailable, got {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn a_robots_txt_redirect_loop_is_unavailable_not_a_hang() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/loop-b"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/loop-b"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/robots.txt"))
            .mount(&server)
            .await;

        let prober = test_prober();
        let origin = origin_of(&server).await;
        // If this hangs or overflows the stack, the test itself never
        // returns — the assertion below only matters once we know we got
        // here at all.
        match prober.check_robots(&origin, "/anything", false).await {
            RobotsDecision::Unavailable(_) => {}
            other => panic!("expected Unavailable for a redirect loop, got {other:?}"),
        }
    }

    /// `validate_hop` is the exact call `fetch_robots_rules` makes before
    /// sending a request on EVERY hop — hop 0 (the initial robots.txt
    /// request) and every redirect target alike, since the loop just
    /// reassigns `current` and calls the same function again. Exercising it
    /// directly against a redirect-shaped private-address URL (the classic
    /// cloud-metadata SSRF target used throughout this crate's tests) proves
    /// that whichever hop a Location header points there, it gets rejected
    /// before any request is sent — without needing a live server actually
    /// listening at a non-public address, which no test in this suite can
    /// arrange (see the `fetch.rs` module doc: `wiremock` only ever binds
    /// loopback).
    #[tokio::test]
    async fn validate_hop_rejects_a_redirect_target_at_a_private_address() {
        let prober = test_prober();
        let target = url::Url::parse("http://169.254.169.254/robots.txt").unwrap();
        let err = prober
            .validate_hop(&target)
            .await
            .expect_err("a link-local / cloud-metadata address must never validate");
        assert!(
            !err.is_empty(),
            "the rejection reason should be non-empty, human-readable text"
        );
    }

    /// Integration-level companion to the test above: with
    /// `validate_hops: true`, `check_robots` must refuse to send even the
    /// FIRST request when that hop's host is private — `wiremock` only ever
    /// binds loopback, so this doubles as the one test in this module that
    /// exercises the real netguard end to end. No mock is mounted on this
    /// server at all: if the guard were bypassed, `guarded_send` would reach
    /// it and get `wiremock`'s default 404 (→ `Open`), not `Unavailable`; and
    /// asserting zero received requests proves nothing was even attempted —
    /// the same outcome a redirect hop landing on a private address gets,
    /// since `fetch_robots_rules` validates every hop through this identical
    /// call site.
    #[tokio::test]
    async fn a_private_hop_is_blocked_before_any_request_is_sent() {
        let server = MockServer::start().await;
        let prober = test_prober();
        let origin = origin_of(&server).await;

        match prober.check_robots(&origin, "/anything", true).await {
            RobotsDecision::Unavailable(reason) => {
                assert!(
                    reason.starts_with("ssrf_blocked:"),
                    "expected an ssrf_blocked reason, got {reason:?}"
                );
            }
            other => panic!("expected Unavailable(ssrf_blocked: ...), got {other:?}"),
        }
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "the guard must reject before any request is sent"
        );
    }

    #[test]
    fn parse_disallow_unions_our_token_and_wildcard_groups() {
        let body = "User-agent: agentcount-probe\nDisallow: /no-probe/\n\nUser-agent: *\nDisallow: /no-anyone/\n";
        let rules = parse_disallow(body);
        assert!(rules.contains(&"/no-probe/".to_string()));
        assert!(rules.contains(&"/no-anyone/".to_string()));
    }

    #[test]
    fn parse_disallow_ignores_groups_for_other_agents() {
        let body = "User-agent: SomeOtherBot\nDisallow: /everything/\n";
        assert!(parse_disallow(body).is_empty());
    }
}
