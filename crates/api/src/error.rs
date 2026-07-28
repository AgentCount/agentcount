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
