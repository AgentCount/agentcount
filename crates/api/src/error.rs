//! One application error type that knows how to become an HTTP response.
//!
//! Handlers do fallible things — query the database, render a template. Rather
//! than each handler hand-rolling error handling, they all return
//! `Result<T, ApiError>` and use `?`. The magic is the `IntoResponse` impl: axum
//! uses it to turn any `ApiError` into a proper HTTP response, so one dropped
//! connection yields a clean 500, not a crashed server.
//!
//! Rust concept spotlight: **implementing traits to plug into a framework.** axum
//! knows nothing about *our* error type — but it knows the `IntoResponse` trait,
//! and `?` knows the `From` trait. By implementing those for `ApiError`, our type
//! slots neatly into both. Traits are Rust's main tool for this kind of
//! "conform to an interface" extensibility.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Everything a handler might fail with.
#[derive(Debug)]
pub enum ApiError {
    /// The requested resource (e.g. a run id or an agent id) doesn't exist → 404.
    NotFound,
    /// A malformed or out-of-range request parameter (an unparsable run id, a
    /// rung outside 1-7, a status that isn't one of the four the schema
    /// allows) → 400. Distinct from `Internal`: the client can fix this by
    /// changing the request, so it must not be logged as if OUR code broke.
    BadRequest(String),
    /// A rate limit was hit → 429, with a `Retry-After` in whole seconds.
    ///
    /// Added 2026-08-03 with the on-demand spot check, which is the only
    /// endpoint that can turn one request from one stranger into a request to
    /// a THIRD party's server. A 429 without `Retry-After` is an invitation to
    /// retry immediately, which is the opposite of what a limit protecting
    /// somebody else's server needs — so the header is part of the variant
    /// rather than something a handler might forget to attach.
    ///
    /// `message` is deliberately safe to show a caller: it names which limit
    /// bound (the caller's own, or the target host's) without reporting counts,
    /// remaining budget, or anything about other callers.
    TooManyRequests {
        retry_after_secs: u64,
        message: String,
    },
    /// The endpoint exists but this deployment cannot serve it → 503.
    ///
    /// Distinct from `Internal`: nothing is broken and nothing failed, a
    /// capability was simply not configured — the spot check with no RPC URL
    /// for the requested chain, for instance. A 500 there would send somebody
    /// hunting a bug that does not exist.
    Unavailable(String),
    /// Anything unexpected (a failed query, a render error) → 500. We keep the
    /// detail server-side and show the client something generic.
    Internal(String),
}

/// A convenient alias so handlers can write `-> ApiResult<Json<...>>`.
pub type ApiResult<T> = Result<T, ApiError>;

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::BadRequest(detail) => (StatusCode::BAD_REQUEST, detail),
            // Returns early: this is the one variant that carries a header, and
            // `Retry-After` is not decoration — it is the whole difference
            // between a limit that protects a third party and one that just
            // provokes a retry loop.
            ApiError::TooManyRequests {
                retry_after_secs,
                message,
            } => {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    [(
                        axum::http::header::RETRY_AFTER,
                        retry_after_secs.to_string(),
                    )],
                    message,
                )
                    .into_response();
            }
            ApiError::Unavailable(detail) => (StatusCode::SERVICE_UNAVAILABLE, detail),
            ApiError::Internal(detail) => {
                // Log the real detail for us; return something bland to the client
                // so we never leak internals.
                tracing::error!("internal error: {detail}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
        };
        (status, message).into_response()
    }
}

// ── `From` conversions so `?` "just works" in handlers ───────────────────────
// When a handler writes `some_query().await?`, `?` converts the error into
// `ApiError` using these `From` impls.

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            // A query that expected exactly one row but found none is a 404.
            sqlx::Error::RowNotFound => ApiError::NotFound,
            other => ApiError::Internal(other.to_string()),
        }
    }
}
