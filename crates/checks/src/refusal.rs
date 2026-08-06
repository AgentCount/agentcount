//! The one definition of "the origin declined us", shared by rungs 2 and 6.
//!
//! Both rungs judge observations from the same prober against the same servers,
//! and the module docs of both say the two must never disagree about who is at
//! fault for the same string. That promise is only keepable if the predicate
//! lives in one place — a second copy is a second answer waiting to happen — so
//! neither rung matches on a status code or an error prefix directly; both call
//! the two functions below.
//!
//! # Where the line is drawn, and why it is drawn there
//!
//! [`declined_us`] admits exactly five HTTP statuses, in two groups that the
//! HTTP specification itself separates:
//!
//! * **429 and 503** are the two statuses defined to carry `Retry-After` (RFC
//!   6585 §4 and RFC 9110 §15.6.4). Both say "not now", about *this request at
//!   this moment*. A 429 in particular is a statement about the requester —
//!   frequently about traffic we generated ourselves — and says nothing at all
//!   about whether the document exists.
//! * **401, 402 and 407** answer with a way in rather than an absence: a
//!   `WWW-Authenticate` challenge, a payment challenge, a `Proxy-Authenticate`
//!   challenge. Something is there, it understood the request, and it wants
//!   credentials or money first.
//!
//! Everything else that is not 2xx stays the agent's `fail`, and three
//! exclusions are worth naming because they look like near misses:
//!
//! * **403 is not here.** It refuses without offering a way in, which is
//!   indistinguishable from "this resource is not available to third parties" —
//!   and that is a fact about the document, which is rung 2's `fail`. A 403
//!   from a bot filter and a 403 from a deliberately private file look
//!   identical on the wire, and guessing between them would be a judgment
//!   dressed as a classification.
//! * **502 and 504 are not here.** A broken upstream means the document really
//!   is not being served to anyone right now; unlike a 503 the origin is not
//!   asking us to come back, it is failing.
//! * **500 is not here**, for the same reason.
//!
//! [`could_not_ask`] covers the other half: `robots.txt` told us not to, or we
//! could not establish permission from it at all. We honour both by sending no
//! request — see `METHODOLOGY.md` §6 — which means we never learned anything
//! about the document. That is not a malfunction of ours (`error`) and not a
//! fact about the agent (`fail`); it is the origin declining, through the one
//! channel the web has for declining.

/// Does this HTTP status mean "something is here and it declined this request"?
///
/// See the module doc for the five statuses and the three near misses.
pub fn declined_us(http_status: u16) -> bool {
    matches!(http_status, 401 | 402 | 407 | 429 | 503)
}

/// Does this prober error mean "we were not given permission to ask"?
///
/// Matches the two `robots.txt` outcomes `crates/probe` produces —
/// `robots_disallowed` (an explicit `Disallow` for our product token or `*`)
/// and `robots_unavailable: …` (a 5xx, a timeout, a connection failure, a
/// redirect loop, or a body that was not UTF-8). Prefix matching, because the
/// unavailable case carries its reason verbatim after the colon and that reason
/// is evidence, not a category.
pub fn could_not_ask(error: &str) -> bool {
    error.starts_with("robots_disallowed") || error.starts_with("robots_unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_five_declining_statuses_and_nothing_else() {
        for code in [401u16, 402, 407, 429, 503] {
            assert!(declined_us(code), "{code} should count as a decline");
        }
        // The near misses named in the module doc, plus the ordinary failures.
        for code in [
            200u16, 204, 301, 400, 403, 404, 410, 500, 502, 504, 521, 530,
        ] {
            assert!(!declined_us(code), "{code} must NOT count as a decline");
        }
    }

    #[test]
    fn a_403_is_not_a_decline_because_it_offers_no_way_in() {
        // Pinned separately from the loop above because it is the case most
        // likely to be re-argued: a 403 from a bot filter and a 403 on a
        // deliberately private file are identical on the wire.
        assert!(!declined_us(403));
    }

    #[test]
    fn both_robots_outcomes_count_and_nothing_else_does() {
        assert!(could_not_ask("robots_disallowed"));
        assert!(could_not_ask(
            "robots_unavailable: robots.txt returned HTTP 503"
        ));
        assert!(could_not_ask(
            "robots_unavailable: connection failed fetching robots.txt: os error 54"
        ));
        assert!(could_not_ask(
            "robots_unavailable: timeout fetching robots.txt"
        ));

        for other in [
            "timeout",
            "tls",
            "connection_failed: dns error",
            "ssrf_blocked: resolves to a non-public address",
            "too_many_redirects",
            "ipfs_all_gateways_failed",
        ] {
            assert!(!could_not_ask(other), "{other} must not read as robots");
        }
    }
}
