use axum::{
    http::{StatusCode, header},
    response::Response,
};
use sqlx::SqlitePool;

use crate::db::models::User;
use crate::db::users;

/// Parse HTTP Basic Auth header and verify credentials.
async fn parse_basic_auth(pool: &SqlitePool, header: &str) -> Result<Option<User>, ()> {
    let encoded = header.strip_prefix("Basic ").ok_or(())?;
    let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        .map_err(|_| ())?;
    let credentials = String::from_utf8(decoded).map_err(|_| ())?;
    let (username, password) = credentials.split_once(':').ok_or(())?;

    users::verify_user(pool, username, password)
        .await
        .map_err(|_| ())
}

/// Build a 401 Unauthorized response with WWW-Authenticate header.
/// Includes DAV headers so Apple Calendar's accountsd recognizes this as
/// a CalDAV server and prompts for credentials.
pub fn unauthorized_response_fn() -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(header::WWW_AUTHENTICATE, "Basic realm=\"CalDAV\"")
        .header("DAV", "1, 2, 3, calendar-access, calendar-schedule")
        .body(axum::body::Body::from("Unauthorized"))
        .unwrap()
}

/// Try to authenticate from a raw Authorization header value.
/// Returns the User if valid, None otherwise.
pub async fn try_basic_auth(pool: &SqlitePool, header: &str) -> Option<User> {
    parse_basic_auth(pool, header).await.ok().flatten()
}
