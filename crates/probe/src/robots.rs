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
//! Parsing itself is deliberately small: we honour `Disallow` for our exact
//! product token and for `*` (unioned, not "most specific wins" — the brief
//! asks for both to be honoured, not for full RFC 9309 precedence), and we do
//! not implement `Allow:` overrides. Good enough for politeness; not a
//! general-purpose robots.txt parser.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, OnceCell};

use crate::fetch::{Prober, SendError};

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
    /// same origin reuses the cached decision.
    pub(crate) async fn check_robots(&self, origin: &url::Url, path: &str) -> RobotsDecision {
        let key = origin.origin().ascii_serialization();
        let cell = {
            let mut entries = self.robots.entries.lock().await;
            entries
                .entry(key)
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone()
        };
        let rules = cell.get_or_init(|| self.fetch_robots_rules(origin)).await;
        apply_rules(rules, path)
    }

    async fn fetch_robots_rules(&self, origin: &url::Url) -> RobotsRules {
        let mut robots_url = origin.clone();
        robots_url.set_path("/robots.txt");
        robots_url.set_query(None);

        match self.guarded_send(&robots_url).await {
            Ok(resp) if (200..300).contains(&resp.status) => match String::from_utf8(resp.body) {
                Ok(text) => RobotsRules::Restricted(parse_disallow(&text)),
                Err(_) => {
                    // A non-UTF-8 robots.txt is unusable, not a permission
                    // grant or denial we can trust — treat like any other
                    // "could not establish permission" case.
                    RobotsRules::Unavailable("robots.txt was not valid UTF-8".into())
                }
            },
            Ok(resp) if (400..500).contains(&resp.status) => RobotsRules::Open,
            Ok(resp) => {
                RobotsRules::Unavailable(format!("robots.txt returned HTTP {}", resp.status))
            }
            Err(SendError::Timeout) => {
                RobotsRules::Unavailable("timeout fetching robots.txt".into())
            }
            Err(SendError::Connection(e)) => {
                RobotsRules::Unavailable(format!("connection failed fetching robots.txt: {e}"))
            }
        }
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
            prober.check_robots(&origin, "/public/card.json").await,
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
            prober.check_robots(&origin, "/private/card.json").await,
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
            prober.check_robots(&origin, "/anything").await,
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
        match prober.check_robots(&origin, "/anything").await {
            RobotsDecision::Unavailable(_) => {}
            other => panic!("expected Unavailable (disallowed-with-error), got {other:?}"),
        }
    }

    #[test]
    fn parse_disallow_unions_our_token_and_wildcard_groups() {
        let body = "User-agent: ledgerscope-probe\nDisallow: /no-probe/\n\nUser-agent: *\nDisallow: /no-anyone/\n";
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
