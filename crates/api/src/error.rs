//! One application error type that knows how to become an HTTP response.
//!
//! Handlers do fallible things — query the database, parse input, call the
//! scorer. Rather than each handler hand-rolling error handling, they all return
//! `Result<T, ApiError>` and use `?`. The magic is the `IntoResponse` impl at
//! the bottom: axum uses it to turn any `ApiError` into a proper HTTP response,
//! so one dropped connection yields a clean `500`, not a crashed server.
//!
//! Rust concept spotlight: **implementing a trait to plug into a framework.**
//! axum doesn't know about *our* error type — but it knows the `IntoResponse`
//! *trait*. By implementing that trait for `ApiError`, we make our type usable
//! anywhere axum expects a response. Traits are Rust's main tool for this kind of
//! "conform to an interface" extensibility.

use axum::response::{IntoResponse, Response};
use axum::http::StatusCode;

/// Everything a handler might fail with. `thiserror` would work here too, but a
/// hand-written enum keeps the mapping-to-HTTP explicit and easy to read.
#[derive(Debug)]
pub enum ApiError {
    /// The requested resource (e.g. an agent id) doesn't exist → 404.
    NotFound,
    /// The client sent something malformed → 400.
    BadRequest(String),
    /// Anything unexpected (a failed query, a scoring bug) → 500. We keep the
    /// detail server-side and show the client something generic.
    Internal(String),
}

/// Turn our error into an HTTP response. This is the trait impl axum calls.
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // A `match` maps each variant to a (status code, message) pair. Because
        // the compiler checks the match is exhaustive, adding a new error variant
        // later forces you to decide its HTTP status — you can't forget.
        let (status, message) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Internal(detail) => {
                // Log the real detail for ourselves, return something bland to
                // the client so we don't leak internals.
                tracing::error!("internal error: {detail}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
            }
        };
        // `(StatusCode, String)` already implements `IntoResponse`, so we lean on
        // that instead of building a `Response` by hand.
        (status, message).into_response()
    }
}

// ── Ergonomic conversions so `?` "just works" in handlers ────────────────────
// When a handler writes `some_query().await?`, the `?` needs to convert the
// query's error type into `ApiError`. Implementing `From<E>` for `ApiError` is
// what makes that automatic. Uncomment/extend these as you wire in real deps.
//
//     impl From<sqlx::Error> for ApiError {
//         fn from(e: sqlx::Error) -> Self {
//             match e {
//                 // A query that expected exactly one row but found none is a 404.
//                 sqlx::Error::RowNotFound => ApiError::NotFound,
//                 other => ApiError::Internal(other.to_string()),
//             }
//         }
//     }
//
//     impl From<scoring::ScoringError> for ApiError {
//         fn from(e: scoring::ScoringError) -> Self {
//             ApiError::Internal(e.to_string())
//         }
//     }

/// A convenient alias so handlers can write `-> ApiResult<Json<...>>`.
pub type ApiResult<T> = Result<T, ApiError>;
