use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sqlx::SqlitePool;

use super::auth::McpUserId;
use super::handlers;
use super::jsonrpc::{JsonRpcRequest, PARSE_ERROR};
use super::session::SessionManager;

/// Shared state for the MCP server.
#[derive(Clone)]
pub struct McpState {
    pub pool: SqlitePool,
    pub sessions: SessionManager,
    pub tool_mode: String,
}

/// Handle POST /mcp — receive JSON-RPC messages from the client.
pub async fn handle_post(State(state): State<McpState>, request: Request<Body>) -> Response {
    let user_id = request
        .extensions()
        .get::<McpUserId>()
        .map(|u| u.0.clone())
        .unwrap_or_default();

    let body = match axum::body::to_bytes(request.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Request body too large").into_response();
        }
    };

    let rpc_request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(e) => {
            let error = serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {"code": PARSE_ERROR, "message": format!("Parse error: {e}")}
            });
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&error).unwrap()))
                .unwrap();
        }
    };

    // Handle notifications (no id) — return 202 Accepted
    if rpc_request.id.is_none() {
        // Still process the notification
        handlers::handle_request(
            &state.pool,
            &state.sessions,
            &user_id,
            &rpc_request,
            &state.tool_mode,
        )
        .await;
        return (StatusCode::ACCEPTED, "").into_response();
    }

    let response = handlers::handle_request(
        &state.pool,
        &state.sessions,
        &user_id,
        &rpc_request,
        &state.tool_mode,
    )
    .await;

    let mut http_response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json");

    // Include session ID header if we just created one
    if rpc_request.method == "initialize"
        && let Some(session_id) = state.sessions.get_user_id(&user_id)
    {
        http_response = http_response.header("Mcp-Session-Id", session_id);
    }

    http_response
        .body(Body::from(serde_json::to_vec(&response).unwrap()))
        .unwrap()
}

/// Handle GET /mcp — SSE stream for server-initiated messages.
/// For our simple server, we just keep the connection open.
pub async fn handle_get() -> Response {
    // For now, return 200 with an empty SSE stream
    // A full implementation would keep this open for server-push notifications
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::empty())
        .unwrap()
}

/// Handle DELETE /mcp — terminate a session.
pub async fn handle_delete(State(state): State<McpState>, request: Request<Body>) -> Response {
    if let Some(session_id) = request
        .headers()
        .get("Mcp-Session-Id")
        .and_then(|v| v.to_str().ok())
    {
        state.sessions.remove_session(session_id);
    }

    (StatusCode::OK, "Session terminated").into_response()
}
