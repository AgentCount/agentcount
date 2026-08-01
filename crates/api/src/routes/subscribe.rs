//! `POST /api/subscribe` — record an address that asked for the reports.
//!
//! The one endpoint in this API that stores something a person typed, and the
//! only one that writes a row about anybody. That makes it the endpoint most
//! worth being careful with, so the reasoning is here rather than spread
//! across a handler.
//!
//! ## What it does NOT do
//!
//! * **It does not confirm the address.** Nothing in this project sends mail,
//!   so nothing can run a double opt-in, so anyone can type anyone else's
//!   address into the form. Every row is written with `confirmed_at` NULL and
//!   migration 0017 says in a column comment not to send to one. Confirmation
//!   is a thing the first send has to do, not a thing this endpoint can fake.
//! * **It does not store an IP address, a user agent, or a referrer.** The
//!   rate limiter below reads the requesting IP and keeps it in memory only.
//!   Writing it would be collecting a second category of personal data for no
//!   stated purpose.
//! * **It does not tell the caller whether an address was already on the
//!   list.** `ON CONFLICT DO NOTHING` and one response either way, so this
//!   endpoint cannot be used to test whether a given person subscribed. That
//!   is a real, cheap enumeration oracle and it costs nothing to close.
//!
//! ## Abuse
//!
//! A public endpoint that writes rows will be found. Three defences, in
//! increasing order of how much they actually help:
//!
//! 1. **A honeypot field.** The form renders a `website` input that is hidden
//!    from people and irresistible to naive bots. Anything that fills it gets
//!    the same success response and no row — telling a bot it failed only
//!    teaches it to try again differently.
//! 2. **Shape validation.** One `@`, something either side, a dot in the
//!    domain, length caps. Not RFC 5322 — that grammar accepts things no
//!    mail server does, and the point here is to reject obvious junk cheaply.
//! 3. **A per-IP window.** In memory, so it is per instance and resets on
//!    deploy. On a scale-to-zero platform with several instances that is
//!    genuinely weak, and it is stated rather than dressed up: it raises the
//!    cost of casual abuse and would not stop anyone determined. The real
//!    backstop is that the table is cheap to clean and the list is worthless
//!    until confirmed.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::Form;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::ApiResult;

/// Long enough for any real address, short enough that the column is not a
/// place to put a paragraph. The practical maximum for an email address is
/// 254 octets; this is that, rounded.
const MAX_EMAIL_LEN: usize = 254;

/// Per-IP submissions allowed in [`RATE_WINDOW`].
///
/// Five, not one: a shared office NAT, a mobile carrier CGNAT, or one person
/// fixing a typo are all normal and none of them should be refused.
const RATE_LIMIT: usize = 5;
const RATE_WINDOW: Duration = Duration::from_secs(600);

#[derive(Debug, Deserialize)]
pub struct SubscribeForm {
    pub email: String,
    /// Which page the form was on. Free text from the client, so it is
    /// truncated and never trusted for anything but a rough tally.
    #[serde(default)]
    pub source: Option<String>,
    /// The honeypot. A person never sees this field; a bot fills every field
    /// it finds.
    #[serde(default)]
    pub website: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SubscribeResponse {
    pub ok: bool,
}

/// Accept an address, or quietly decline to.
///
/// Returns 200 for a stored address, for a duplicate, and for a honeypot hit
/// — all three are "nothing more for you to do here" from the caller's side,
/// and distinguishing them would leak either list membership or the honeypot.
/// 400 is reserved for an address that is not shaped like one, because that IS
/// the submitter's problem and they can fix it. 429 when the window is full.
pub async fn post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<SubscribeForm>,
) -> ApiResult<(StatusCode, axum::Json<SubscribeResponse>)> {
    // The honeypot, checked before anything expensive. Success-shaped on
    // purpose.
    if form
        .website
        .as_deref()
        .is_some_and(|w| !w.trim().is_empty())
    {
        tracing::debug!("honeypot triggered on /api/subscribe");
        return Ok((StatusCode::OK, axum::Json(SubscribeResponse { ok: true })));
    }

    let Some(email) = normalise_email(&form.email) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            axum::Json(SubscribeResponse { ok: false }),
        ));
    };

    if !allow(client_ip(&headers).as_deref()) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(SubscribeResponse { ok: false }),
        ));
    }

    let source = form.source.as_deref().map(|s| {
        let s = s.trim();
        &s[..s.len().min(64)]
    });

    // `DO NOTHING`, so re-subscribing is a no-op rather than an error and
    // never moves `subscribed_at`. Someone who previously unsubscribed stays
    // unsubscribed until they say otherwise through a path that does not exist
    // yet — resurrecting them silently here would be the worst possible
    // behaviour of the three.
    sqlx::query(
        "INSERT INTO newsletter_subscribers (email, source) \
         VALUES ($1, $2) ON CONFLICT (email) DO NOTHING",
    )
    .bind(&email)
    .bind(source)
    .execute(&state.db)
    .await?;

    Ok((StatusCode::OK, axum::Json(SubscribeResponse { ok: true })))
}

/// Lowercase, trim, and reject anything not shaped like an address.
///
/// Deliberately not RFC 5322: that grammar permits quoted strings, comments
/// and addresses no mail server would accept, and implementing it would reject
/// nothing extra that matters while adding a parser to maintain. This checks
/// what a typo actually looks like.
fn normalise_email(raw: &str) -> Option<String> {
    let e = raw.trim().to_ascii_lowercase();
    if e.is_empty() || e.len() > MAX_EMAIL_LEN {
        return None;
    }
    // No whitespace anywhere, and no control characters — either means the
    // value was assembled rather than typed.
    if e.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let (local, domain) = e.split_once('@')?;
    if local.is_empty() || domain.is_empty() {
        return None;
    }
    // Exactly one `@`.
    if domain.contains('@') {
        return None;
    }
    // A domain with a dot, not starting or ending with one, and no empty label.
    if !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || domain.contains("..")
    {
        return None;
    }
    Some(e)
}

/// The requesting address, from the proxy header the platform sets.
///
/// `X-Forwarded-For` is client-supplied and trivially spoofed in general — the
/// leftmost entry is whatever the client claimed. Behind Cloud Run's frontend
/// the *last* entry is the one Google observed, so that is the one taken.
/// Nothing security-critical rests on it; it only keys a rate limiter.
fn client_ip(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get("x-forwarded-for")?.to_str().ok()?;
    raw.rsplit(',').next().map(|s| s.trim().to_string())
}

/// The window. A `Mutex<HashMap>` rather than anything cleverer because this
/// endpoint is not hot, and a lock held for a map lookup is cheaper than the
/// database call it guards.
static SEEN: Mutex<Option<HashMap<String, Vec<Instant>>>> = Mutex::new(None);

fn allow(ip: Option<&str>) -> bool {
    // No header means we cannot tell callers apart. Allowing is the right
    // failure: refusing everyone because a proxy header is missing would take
    // the form down for a configuration detail.
    let Some(ip) = ip else { return true };
    let Ok(mut guard) = SEEN.lock() else {
        // A poisoned lock means another thread panicked while holding it. The
        // rate limiter failing open is much better than the endpoint failing.
        return true;
    };
    let map = guard.get_or_insert_with(HashMap::new);
    let now = Instant::now();

    // Opportunistic sweep, so a long-lived instance does not accumulate an
    // entry per address that ever called. Cheap because the map is small.
    map.retain(|_, hits| {
        hits.retain(|t| now.duration_since(*t) < RATE_WINDOW);
        !hits.is_empty()
    });

    let hits = map.entry(ip.to_string()).or_default();
    if hits.len() >= RATE_LIMIT {
        return false;
    }
    hits.push(now);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plausible_addresses_are_lowercased_and_kept() {
        for (raw, want) in [
            ("filip@example.com", "filip@example.com"),
            ("  Filip@Example.COM  ", "filip@example.com"),
            ("a.b+tag@sub.example.co.uk", "a.b+tag@sub.example.co.uk"),
        ] {
            assert_eq!(normalise_email(raw).as_deref(), Some(want), "{raw:?}");
        }
    }

    #[test]
    fn the_same_address_in_two_cases_is_one_subscriber() {
        // The property that matters: it decides whether somebody gets mailed
        // once or twice, and the primary key can only enforce it if the
        // boundary normalises first.
        assert_eq!(
            normalise_email("Filip@Example.com"),
            normalise_email("filip@example.com")
        );
    }

    #[test]
    fn junk_is_rejected() {
        for raw in [
            "",
            "   ",
            "filip",
            "@example.com",
            "filip@",
            "filip@example",       // no dot in the domain
            "filip@.com",          // leading dot
            "filip@example.",      // trailing dot
            "filip@exa..mple.com", // empty label
            "a@b@example.com",     // two @
            "filip @example.com",  // whitespace
            "filip@example.com\n", // trailing newline is trimmed, but…
        ] {
            let got = normalise_email(raw);
            // The last case trims to something valid — assert that explicitly
            // rather than let it silently weaken the loop.
            if raw == "filip@example.com\n" {
                assert_eq!(got.as_deref(), Some("filip@example.com"));
            } else {
                assert!(got.is_none(), "{raw:?} should be rejected, got {got:?}");
            }
        }
    }

    #[test]
    fn a_header_injection_attempt_is_rejected_rather_than_stored() {
        // Not because this endpoint sends mail — it does not — but because the
        // row it writes is what a later sender will read, and a newline in an
        // address is how header injection starts.
        assert!(normalise_email("filip@example.com\r\nBcc: victim@example.com").is_none());
    }

    #[test]
    fn an_over_long_address_is_rejected() {
        let long = format!("{}@example.com", "a".repeat(300));
        assert!(normalise_email(&long).is_none());
    }

    #[test]
    fn the_observed_ip_is_taken_from_the_right_end_of_the_chain() {
        // The leftmost entry is whatever the client claimed; the rightmost is
        // what the platform's own proxy observed. Keying a limiter on the
        // client-controlled end would make it trivially bypassable.
        let mut h = HeaderMap::new();
        h.insert(
            "x-forwarded-for",
            "1.2.3.4, 5.6.7.8, 9.10.11.12".parse().unwrap(),
        );
        assert_eq!(client_ip(&h).as_deref(), Some("9.10.11.12"));

        let mut single = HeaderMap::new();
        single.insert("x-forwarded-for", "1.2.3.4".parse().unwrap());
        assert_eq!(client_ip(&single).as_deref(), Some("1.2.3.4"));

        assert_eq!(client_ip(&HeaderMap::new()), None);
    }

    #[test]
    fn the_window_lets_a_few_through_then_refuses() {
        let ip = "203.0.113.99";
        for i in 0..RATE_LIMIT {
            assert!(allow(Some(ip)), "submission {i} should be allowed");
        }
        assert!(!allow(Some(ip)), "the window should now be full");
        // A different caller is unaffected.
        assert!(allow(Some("203.0.113.100")));
    }

    #[test]
    fn a_missing_proxy_header_fails_open() {
        // Refusing everyone because a header is absent would take the form
        // down for a configuration detail.
        assert!(allow(None));
    }
}
