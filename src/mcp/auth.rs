use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use sqlx::SqlitePool;

use crate::db::tokens;

/// Middleware to require Bearer token authentication for MCP requests.
/// On success, inserts the user_id into request extensions.
pub async fn require_bearer_auth(
    State(pool): State<SqlitePool>,
    mut request: Request,
    next: Next,
) -> Result<Response, Response> {
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| unauthorized_response("Missing Authorization header"))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| unauthorized_response("Invalid authorization scheme, expected Bearer"))?;

    let user_id = tokens::validate_token(&pool, token)
        .await
        .map_err(|_| unauthorized_response("Token validation failed"))?
        .ok_or_else(|| unauthorized_response("Invalid or expired token"))?;

    // Store user_id in request extensions
    request.extensions_mut().insert(McpUserId(user_id));

    Ok(next.run(request).await)
}

/// Wrapper for the authenticated MCP user's ID.
#[derive(Debug, Clone)]
pub struct McpUserId(pub String);

fn unauthorized_response(msg: &str) -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(
            header::WWW_AUTHENTICATE,
            "Bearer realm=\"CalDAV MCP\"",
        )
        .body(axum::body::Body::from(msg.to_string()))
        .unwrap()
}
