//! The production `DATABASE_URL` format must parse.
//!
//! On Cloud Run the database is reached over a **unix socket** mounted at
//! `/cloudsql/<project>:<region>:<instance>`, never over the network — which is
//! what keeps Postgres off the public internet. Getting that into a URL is
//! fiddly in a way that fails late and unhelpfully.
//!
//! The libpq spelling — `postgres://user:pass@/db?host=/cloudsql/...` — looks
//! obviously right and is what Google's own documentation shows. It does not
//! work here. sqlx builds on the `url` crate, which rejects an authority with
//! an empty host (`@/db`) before sqlx's own `?host=` handling is ever reached,
//! and the resulting message is `error with configuration: empty host`.
//!
//! On Cloud Run that surfaces as neither a database error nor a configuration
//! error, but as:
//!
//!     The user-provided container failed to start and listen on the port
//!     defined provided by the PORT=8080 environment variable
//!
//! because the API connects to Postgres before it binds. So a connection-string
//! typo presents as a port problem, and costs an hour looking at the wrong
//! thing. Hence a test.

use std::str::FromStr;

use sqlx::postgres::PgConnectOptions;

/// The form that must keep working: socket path percent-encoded into the host
/// position (`/` → `%2F`, `:` → `%3A`).
#[test]
fn the_cloud_run_socket_url_parses_and_is_understood_as_a_socket() {
    let url = "postgres://agentcount_api:secret@\
               %2Fcloudsql%2Fproject%3Aregion%3Ainstance/agentcount";

    let opts = PgConnectOptions::from_str(url).expect("production DATABASE_URL form must parse");

    // Debug is the only accessor sqlx exposes for the socket, but asserting on
    // it is still the point: this must be a SOCKET connection, not a TCP one to
    // a host that happens to be named like a path.
    let rendered = format!("{opts:?}");
    assert!(
        rendered.contains(r#"socket: Some("/cloudsql/project:region:instance")"#),
        "expected a decoded unix socket path, got: {rendered}"
    );
    assert!(
        rendered.contains(r#"database: Some("agentcount")"#),
        "database name lost: {rendered}"
    );
}

/// The form that looks right, is what libpq accepts, and silently is not.
/// Pinned so that anyone who "fixes" the URL back to the documented-looking
/// spelling gets a failing test instead of a container that will not start.
#[test]
fn the_libpq_host_parameter_form_is_rejected_and_that_is_why_we_encode() {
    let url = "postgres://agentcount_api:secret@/agentcount\
               ?host=/cloudsql/project:region:instance";

    let err = PgConnectOptions::from_str(url)
        .expect_err("this form must NOT parse — if it starts working, simplify the URL");

    assert!(
        err.to_string().contains("empty host"),
        "expected the empty-host rejection, got: {err}"
    );
}
