use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use sqlx::SqlitePool;

use crate::db::models::User;
use crate::db::users;

/// Extract the authenticated user from the request via HTTP Basic Auth.
/// Returns 401 with WWW-Authenticate header if auth fails.
pub async fn require_auth(
    State(pool): State<SqlitePool>,
    mut request: Request,
    next: Next,
) -> Result<Response, Response> {
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(unauthorized_response)?;

    let user = parse_basic_auth(&pool, auth_header)
        .await
        .map_err(|_| unauthorized_response())?
        .ok_or_else(unauthorized_response)?;

    // Store authenticated user in request extensions
    request.extensions_mut().insert(user);

    Ok(next.run(request).await)
}

/// Parse HTTP Basic Auth header and verify credentials.
async fn parse_basic_auth(pool: &SqlitePool, header: &str) -> Result<Option<User>, ()> {
    let encoded = header.strip_prefix("Basic ").ok_or(())?;
    let decoded = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        encoded,
    )
    .map_err(|_| ())?;
    let credentials = String::from_utf8(decoded).map_err(|_| ())?;
    let (username, password) = credentials.split_once(':').ok_or(())?;

    users::verify_user(pool, username, password)
        .await
        .map_err(|_| ())
}

/// Build a 401 Unauthorized response with WWW-Authenticate header.
fn unauthorized_response() -> Response {
    let mut response = Response::new(axum::body::Body::from("Unauthorized"));
    *response.status_mut() = StatusCode::UNAUTHORIZED;
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        "Basic realm=\"CalDAV\"".parse().unwrap(),
    );
    response
}

