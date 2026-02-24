mod auth;
mod handlers;
mod jsonrpc;
mod session;
mod tools;
mod transport;

use axum::middleware;
use axum::routing::{delete, get, post};
use axum::Router;
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;

use session::SessionManager;
use transport::McpState;

/// Build the MCP router. Mounted on the MCP port.
pub fn router(pool: SqlitePool) -> Router {
    let state = McpState {
        pool: pool.clone(),
        sessions: SessionManager::new(),
    };

    Router::new()
        .route("/mcp", post(transport::handle_post))
        .route("/mcp", get(transport::handle_get))
        .route("/mcp", delete(transport::handle_delete))
        .layer(middleware::from_fn_with_state(
            pool.clone(),
            auth::require_bearer_auth,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
