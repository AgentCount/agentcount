//! Polite HTTP against catalogs, with every response hashed.
//!
//! Three rules from METHODOLOGY §10, enforced here rather than remembered:
//!
//! * **robots.txt binds these requests too** (§10.3), with no carve-out. A
//!   catalog that disallows us is `refused` and its listings are not in this
//!   run's population — which is a fact the run RECORDS, because a
//!   population assembled from four of six catalogs is a different
//!   population.
//! * **Every fetched body is hashed** (§10.2). That hash is what replaces a
//!   pinned block: it is the only way "these were the listings this week"
//!   stays checkable after the catalog has rewritten itself.
//! * **One request at a time per host, with a pause between them.** A
//!   catalog is somebody's server, and this census reads all of it, weekly,
//!   forever.
//!
//! Nothing here judges. A response is bytes plus an outcome word; what those
//! bytes mean is `crates/sellers`' problem.

use std::time::Duration;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::{PRODUCT_TOKEN, USER_AGENT};

/// How long to wait between two requests to the same catalog host.
///
/// 500ms — twice the agent prober's per-host pause, because a catalog is
/// paginated and this census reads every page of it, so the total request
/// count per host is far higher than the one-request-per-agent-document case
/// that number was chosen for.
pub const CATALOG_PAGE_DELAY: Duration = Duration::from_millis(500);

/// Total per-request budget. A catalog that has not answered in this long is
/// not refusing us and is not broken as far as we can tell — it is slow, and
/// the run says `error`, which is OUR word.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The most bytes this census will read from one catalog page. Generous
/// (catalog pages carry embedded JSON schemas), but bounded: a hostile or
/// broken endpoint must not be able to exhaust memory.
pub const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// What one catalog request produced.
#[derive(Debug, Clone)]
pub struct Fetched {
    pub url: String,
    /// `fetched` | `refused` | `error`, the words migration 0026 stores.
    pub outcome: Outcome,
    pub http_status: Option<u16>,
    pub body: Option<String>,
    /// Hex sha256 of the exact bytes received. `None` unless fetched.
    pub sha256: Option<String>,
    pub byte_len: Option<i64>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Fetched,
    /// The origin is there and declined us: robots.txt said no, a rate limit,
    /// an auth challenge. Not our failure and not theirs.
    Refused,
    /// OUR failure: a timeout, a TLS error, a body we could not read.
    Error,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fetched => "fetched",
            Self::Refused => "refused",
            Self::Error => "error",
        }
    }
}

/// A client that honours robots.txt and hashes what it reads.
pub struct CatalogFetcher {
    http: reqwest::Client,
    prober: probe::Prober,
}

impl CatalogFetcher {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .timeout(REQUEST_TIMEOUT)
                .build()
                .context("building the catalog HTTP client")?,
            // The robots.txt implementation this project already has, asked
            // under THIS instrument's product token: a catalog operator can
            // disallow `agentcount-sellers` without touching the agent
            // prober, and a rule for `*` still applies to both.
            prober: probe::Prober::for_robots_only(
                PRODUCT_TOKEN,
                "https://agentcount.ai/methodology; contact: census@agentcount.ai",
            )
            .context("building the robots.txt client")?,
        })
    }

    /// Fetch one catalog URL, after asking its host for permission.
    pub async fn get(&self, url: &str) -> Fetched {
        let parsed = match url::Url::parse(url) {
            Ok(u) => u,
            Err(e) => return Fetched::error(url, format!("unparseable url: {e}")),
        };

        // Permission first, always. A request we were told not to send is
        // not sent, and the run says so.
        match self.prober.robots_permits(&parsed).await {
            probe::RobotsDecision::Allowed => {}
            probe::RobotsDecision::Disallowed => {
                return Fetched::refused(url, None, "robots_disallowed");
            }
            probe::RobotsDecision::Unavailable(reason) => {
                // RFC 9309 §2.3.1.4: an unreachable robots.txt is read as
                // disallow. We could not establish permission, so we do not
                // ask — and `refused` names that honestly, per the same
                // 2026-08-06 ruling the agent census made.
                return Fetched::refused(url, None, &format!("robots_unavailable: {reason}"));
            }
        }

        let response = match self.http.get(url).send().await {
            Ok(r) => r,
            Err(e) => return Fetched::error(url, format!("request failed: {e}")),
        };
        let status = response.status();

        // The statuses that mean "the origin is there and declined us",
        // exactly as `checks::refusal` reads them for agents: come-back-later
        // and challenges. Everything else non-2xx is the catalog's own
        // answer and is recorded as an error of the fetch, not a refusal.
        if matches!(status.as_u16(), 401 | 402 | 403 | 407 | 429 | 503) {
            return Fetched::refused(url, Some(status.as_u16()), "declined");
        }

        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => return Fetched::error(url, format!("body read failed: {e}")),
        };
        if bytes.len() > MAX_BODY_BYTES {
            return Fetched::error(url, format!("body over {MAX_BODY_BYTES} bytes"));
        }
        if !status.is_success() {
            return Fetched::error(url, format!("http {}", status.as_u16()));
        }

        // The hash is over the exact bytes received, before any parsing —
        // that is what a later reader can reproduce.
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let body = match String::from_utf8(bytes.to_vec()) {
            Ok(s) => s,
            Err(e) => return Fetched::error(url, format!("body is not utf-8: {e}")),
        };

        Fetched {
            url: url.to_string(),
            outcome: Outcome::Fetched,
            http_status: Some(status.as_u16()),
            byte_len: Some(body.len() as i64),
            sha256: Some(sha256),
            body: Some(body),
            note: None,
        }
    }

    /// The product token a catalog operator would write in their robots.txt
    /// to disallow this instrument specifically.
    pub fn product_token() -> &'static str {
        PRODUCT_TOKEN
    }
}

impl Fetched {
    fn refused(url: &str, status: Option<u16>, note: &str) -> Self {
        Self {
            url: url.to_string(),
            outcome: Outcome::Refused,
            http_status: status,
            body: None,
            sha256: None,
            byte_len: None,
            note: Some(note.to_string()),
        }
    }

    fn error(url: &str, note: String) -> Self {
        Self {
            url: url.to_string(),
            outcome: Outcome::Error,
            http_status: None,
            body: None,
            sha256: None,
            byte_len: None,
            note: Some(note),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_and_an_error_are_different_rows() {
        // The distinction the whole status vocabulary exists to keep: a
        // catalog that declined us is not a catalog we failed to read.
        let refused = Fetched::refused("https://a.example/x", Some(429), "declined");
        let errored = Fetched::error("https://a.example/x", "timeout".into());
        assert_eq!(refused.outcome.as_str(), "refused");
        assert_eq!(errored.outcome.as_str(), "error");
        // Neither carries bytes, and neither may carry a hash — a hash
        // implies we have the bytes it names.
        assert!(refused.sha256.is_none() && errored.sha256.is_none());
        assert!(refused.body.is_none() && errored.body.is_none());
    }

    #[test]
    fn the_outcome_words_are_the_ones_the_schema_stores() {
        // Migration 0026's CHECK constraint accepts exactly these three.
        assert_eq!(Outcome::Fetched.as_str(), "fetched");
        assert_eq!(Outcome::Refused.as_str(), "refused");
        assert_eq!(Outcome::Error.as_str(), "error");
    }

    #[test]
    fn this_instrument_identifies_itself_separately_from_the_agent_prober() {
        // So a catalog operator can disallow one without the other.
        assert_eq!(CatalogFetcher::product_token(), "agentcount-sellers");
        assert!(USER_AGENT.contains("methodology"));
    }
}
