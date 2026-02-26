mod auth;
mod handlers;
mod jsonrpc;
mod session;
mod tools;
mod transport;

use axum::Router;
use axum::middleware;
use axum::routing::{delete, get, post};
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;

use session::SessionManager;
use transport::McpState;

/// Build the MCP router. Mounted on the MCP port.
pub fn router(pool: SqlitePool, tool_mode: String) -> Router {
    let state = McpState {
        pool: pool.clone(),
        sessions: SessionManager::new(),
        tool_mode,
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use crate::db;
    use crate::db::{calendars, tokens, users};

    /// Create a test pool with a user and a valid MCP bearer token.
    async fn setup() -> (SqlitePool, String, String) {
        let pool = db::test_pool().await;
        let user = users::create_user(&pool, "alice", Some("alice@example.com"), "secret123")
            .await
            .unwrap();
        let (raw_token, _record) = tokens::create_token(&pool, &user.id, "test-token")
            .await
            .unwrap();
        (pool, user.id, raw_token)
    }

    fn bearer_header(token: &str) -> String {
        format!("Bearer {token}")
    }

    /// Send a JSON-RPC request to /mcp and return (status, parsed body).
    async fn rpc_call(pool: &SqlitePool, token: &str, body: Value) -> (StatusCode, Value) {
        let app = router(pool.clone(), "full".to_string());
        let req = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header("Content-Type", "application/json")
            .header("Authorization", bearer_header(token))
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    /// Helper: call tools/call and return the structuredContent from the result.
    async fn tool_call(pool: &SqlitePool, token: &str, tool_name: &str, arguments: Value) -> Value {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments
            }
        });
        let (status, resp) = rpc_call(pool, token, body).await;
        assert_eq!(status, StatusCode::OK);
        resp["result"]["structuredContent"].clone()
    }

    // ---- Auth tests ----

    #[tokio::test]
    async fn test_no_auth_returns_401() {
        let pool = db::test_pool().await;
        let app = router(pool, "full".to_string());
        let req = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_invalid_token_returns_401() {
        let pool = db::test_pool().await;
        let app = router(pool, "full".to_string());
        let req = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header("Content-Type", "application/json")
            .header("Authorization", "Bearer mcp_bogus_token")
            .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ---- Protocol tests ----

    #[tokio::test]
    async fn test_initialize() {
        let (pool, _user_id, token) = setup().await;
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "clientInfo": {"name": "test-client", "version": "0.1"}
            }
        });
        let (status, resp) = rpc_call(&pool, &token, body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(resp["result"]["serverInfo"]["name"], "caldav-mcp-server");
    }

    #[tokio::test]
    async fn test_ping() {
        let (pool, _user_id, token) = setup().await;
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
        let (status, resp) = rpc_call(&pool, &token, body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["result"], json!({}));
    }

    #[tokio::test]
    async fn test_tools_list() {
        let (pool, _user_id, token) = setup().await;
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
        let (status, resp) = rpc_call(&pool, &token, body).await;
        assert_eq!(status, StatusCode::OK);
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 12);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"list_calendars"));
        assert!(names.contains(&"create_event"));
        assert!(names.contains(&"share_calendar"));
    }

    #[tokio::test]
    async fn test_unknown_method() {
        let (pool, _user_id, token) = setup().await;
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "nonexistent/method"});
        let (status, resp) = rpc_call(&pool, &token, body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn test_notification_returns_202() {
        let (pool, _user_id, token) = setup().await;
        let app = router(pool, "full".to_string());
        // Notification = no "id" field
        let body = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let req = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header("Content-Type", "application/json")
            .header("Authorization", bearer_header(&token))
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
    }

    // ---- Calendar CRUD via MCP tools ----

    #[tokio::test]
    async fn test_create_and_list_calendar() {
        let (pool, _user_id, token) = setup().await;

        // Create a calendar
        let result = tool_call(
            &pool,
            &token,
            "create_calendar",
            json!({
                "name": "Work",
                "description": "Work events",
                "color": "#FF0000"
            }),
        )
        .await;
        assert_eq!(result["name"], "Work");
        assert_eq!(result["color"], "#FF0000");
        let cal_id = result["id"].as_str().unwrap().to_string();

        // List calendars — should include the new one
        let result = tool_call(&pool, &token, "list_calendars", json!({})).await;
        let cals = result["calendars"].as_array().unwrap();
        assert_eq!(cals.len(), 1);
        assert_eq!(cals[0]["id"], cal_id);
        assert_eq!(cals[0]["name"], "Work");

        // Verify in DB directly
        let db_cal = calendars::get_calendar_by_id(&pool, &cal_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(db_cal.name, "Work");
        assert_eq!(db_cal.color, "#FF0000");
    }

    #[tokio::test]
    async fn test_get_calendar() {
        let (pool, user_id, token) = setup().await;
        let cal = calendars::create_calendar(&pool, &user_id, "Home", "Personal", "#00FF00", "UTC")
            .await
            .unwrap();

        let result = tool_call(
            &pool,
            &token,
            "get_calendar",
            json!({"calendar_id": cal.id}),
        )
        .await;
        assert_eq!(result["name"], "Home");
        assert_eq!(result["description"], "Personal");
        assert_eq!(result["color"], "#00FF00");
    }

    #[tokio::test]
    async fn test_delete_calendar() {
        let (pool, user_id, token) = setup().await;
        let cal = calendars::create_calendar(&pool, &user_id, "Temp", "", "#000", "UTC")
            .await
            .unwrap();

        let result = tool_call(
            &pool,
            &token,
            "delete_calendar",
            json!({"calendar_id": cal.id}),
        )
        .await;
        assert_eq!(result["deleted"], true);

        // Verify gone from DB
        let db_cal = calendars::get_calendar_by_id(&pool, &cal.id).await.unwrap();
        assert!(db_cal.is_none());
    }

    // ---- Event CRUD via MCP tools ----

    #[tokio::test]
    async fn test_create_get_update_delete_event() {
        let (pool, user_id, token) = setup().await;
        let cal = calendars::create_calendar(&pool, &user_id, "Work", "", "#000", "UTC")
            .await
            .unwrap();

        // Create
        let result = tool_call(
            &pool,
            &token,
            "create_event",
            json!({
                "calendar_id": cal.id,
                "title": "Team Standup",
                "start": "20260301T090000Z",
                "end": "20260301T093000Z",
                "description": "Daily sync",
                "location": "Room 42"
            }),
        )
        .await;
        assert_eq!(result["title"], "Team Standup");
        assert_eq!(result["calendar_id"], cal.id);
        let uid = result["uid"].as_str().unwrap().to_string();
        let etag = result["etag"].as_str().unwrap().to_string();

        // Verify in DB
        let db_obj = crate::db::events::get_object_by_uid(&pool, &cal.id, &uid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(db_obj.summary.as_deref(), Some("Team Standup"));
        assert_eq!(db_obj.dtstart.as_deref(), Some("20260301T090000Z"));

        // Get
        let result = tool_call(
            &pool,
            &token,
            "get_event",
            json!({
                "calendar_id": cal.id,
                "event_uid": uid
            }),
        )
        .await;
        assert_eq!(result["summary"], "Team Standup");
        assert_eq!(result["etag"], etag);
        assert!(result["ical_data"].as_str().unwrap().contains("VEVENT"));

        // Update
        let result = tool_call(
            &pool,
            &token,
            "update_event",
            json!({
                "calendar_id": cal.id,
                "event_uid": uid,
                "title": "Team Standup v2",
                "start": "20260301T100000Z",
                "end": "20260301T103000Z"
            }),
        )
        .await;
        assert_eq!(result["updated"], true);
        assert_eq!(result["title"], "Team Standup v2");
        let new_etag = result["etag"].as_str().unwrap().to_string();
        assert_ne!(new_etag, etag, "ETag should change after update");

        // Verify update in DB
        let db_obj = crate::db::events::get_object_by_uid(&pool, &cal.id, &uid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(db_obj.summary.as_deref(), Some("Team Standup v2"));
        assert_eq!(db_obj.dtstart.as_deref(), Some("20260301T100000Z"));

        // Delete
        let result = tool_call(
            &pool,
            &token,
            "delete_event",
            json!({
                "calendar_id": cal.id,
                "event_uid": uid
            }),
        )
        .await;
        assert_eq!(result["deleted"], true);

        // Verify gone from DB
        let db_obj = crate::db::events::get_object_by_uid(&pool, &cal.id, &uid)
            .await
            .unwrap();
        assert!(db_obj.is_none());
    }

    #[tokio::test]
    async fn test_query_events() {
        let (pool, user_id, token) = setup().await;
        let cal = calendars::create_calendar(&pool, &user_id, "Work", "", "#000", "UTC")
            .await
            .unwrap();

        // Create two events
        tool_call(
            &pool,
            &token,
            "create_event",
            json!({
                "calendar_id": cal.id,
                "title": "Morning",
                "start": "20260301T090000Z",
                "end": "20260301T100000Z"
            }),
        )
        .await;
        tool_call(
            &pool,
            &token,
            "create_event",
            json!({
                "calendar_id": cal.id,
                "title": "Afternoon",
                "start": "20260301T140000Z",
                "end": "20260301T150000Z"
            }),
        )
        .await;

        // Query all events
        let result = tool_call(
            &pool,
            &token,
            "query_events",
            json!({"calendar_id": cal.id}),
        )
        .await;
        assert_eq!(result["count"], 2);
        assert_eq!(result["events"].as_array().unwrap().len(), 2);

        // Query with time range — only morning event
        let result = tool_call(
            &pool,
            &token,
            "query_events",
            json!({
                "calendar_id": cal.id,
                "start": "20260301T080000Z",
                "end": "20260301T110000Z"
            }),
        )
        .await;
        assert_eq!(result["count"], 1);
        assert_eq!(result["events"][0]["summary"], "Morning");
    }

    // ---- Sharing via MCP tools ----

    #[tokio::test]
    async fn test_share_and_list_shared_calendars() {
        let pool = db::test_pool().await;
        let alice = users::create_user(&pool, "alice", None, "pass1")
            .await
            .unwrap();
        let bob = users::create_user(&pool, "bob", None, "pass2")
            .await
            .unwrap();
        let (alice_token, _) = tokens::create_token(&pool, &alice.id, "alice-tok")
            .await
            .unwrap();
        let (bob_token, _) = tokens::create_token(&pool, &bob.id, "bob-tok")
            .await
            .unwrap();

        // Alice creates a calendar
        let result = tool_call(
            &pool,
            &alice_token,
            "create_calendar",
            json!({"name": "Shared Cal"}),
        )
        .await;
        let cal_id = result["id"].as_str().unwrap().to_string();

        // Alice shares it with Bob (read-only)
        let result = tool_call(
            &pool,
            &alice_token,
            "share_calendar",
            json!({
                "calendar_id": cal_id,
                "username": "bob",
                "permission": "read"
            }),
        )
        .await;
        assert_eq!(result["shared_with"], "bob");
        assert_eq!(result["permission"], "read");

        // Bob lists shared calendars
        let result = tool_call(&pool, &bob_token, "list_shared_calendars", json!({})).await;
        let shared = result["shared_calendars"].as_array().unwrap();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0]["name"], "Shared Cal");
        assert_eq!(shared[0]["permission"], "read");

        // Alice unshares
        let result = tool_call(
            &pool,
            &alice_token,
            "unshare_calendar",
            json!({
                "calendar_id": cal_id,
                "username": "bob"
            }),
        )
        .await;
        assert_eq!(result["unshared"], true);

        // Bob should see no shared calendars now
        let result = tool_call(&pool, &bob_token, "list_shared_calendars", json!({})).await;
        assert_eq!(result["shared_calendars"].as_array().unwrap().len(), 0);
    }

    // ---- Error handling ----

    #[tokio::test]
    async fn test_tool_call_missing_params() {
        let (pool, _user_id, token) = setup().await;
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {}
        });
        let (status, resp) = rpc_call(&pool, &token, body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn test_tool_call_unknown_tool() {
        let (pool, _user_id, token) = setup().await;
        let result_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": "nonexistent_tool", "arguments": {}}
        });
        let (status, resp) = rpc_call(&pool, &token, result_body).await;
        assert_eq!(status, StatusCode::OK);
        // Tool errors are returned as isError=true, not JSON-RPC errors
        assert_eq!(resp["result"]["isError"], true);
    }

    #[tokio::test]
    async fn test_get_event_not_found() {
        let (pool, user_id, token) = setup().await;
        let cal = calendars::create_calendar(&pool, &user_id, "Work", "", "#000", "UTC")
            .await
            .unwrap();

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "get_event",
                "arguments": {"calendar_id": cal.id, "event_uid": "nonexistent-uid"}
            }
        });
        let (status, resp) = rpc_call(&pool, &token, body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["result"]["isError"], true);
        assert!(
            resp["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("not found")
        );
    }

    #[tokio::test]
    async fn test_invalid_json_returns_parse_error() {
        let (pool, _user_id, token) = setup().await;
        let app = router(pool, "full".to_string());
        let req = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header("Content-Type", "application/json")
            .header("Authorization", bearer_header(&token))
            .body(Body::from("not valid json"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"]["code"], -32700);
    }

    // ---- DELETE session ----

    #[tokio::test]
    async fn test_delete_session() {
        let (pool, _user_id, token) = setup().await;
        let app = router(pool, "full".to_string());
        let req = axum::http::Request::builder()
            .method(Method::DELETE)
            .uri("/mcp")
            .header("Authorization", bearer_header(&token))
            .header("Mcp-Session-Id", "some-session-id")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ==== Simple mode helpers ====

    /// Send a JSON-RPC request in simple mode.
    async fn simple_rpc_call(pool: &SqlitePool, token: &str, body: Value) -> (StatusCode, Value) {
        let app = router(pool.clone(), "simple".to_string());
        let req = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header("Content-Type", "application/json")
            .header("Authorization", bearer_header(token))
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    /// Call a simple-mode tool and return structuredContent.
    async fn simple_tool_call(
        pool: &SqlitePool,
        token: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Value {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": tool_name, "arguments": arguments}
        });
        let (status, resp) = simple_rpc_call(pool, token, body).await;
        assert_eq!(status, StatusCode::OK);
        resp["result"]["structuredContent"].clone()
    }

    // ==== Simple mode tests ====

    #[tokio::test]
    async fn test_simple_tools_list_returns_3() {
        let (pool, _user_id, token) = setup().await;
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
        let (status, resp) = simple_rpc_call(&pool, &token, body).await;
        assert_eq!(status, StatusCode::OK);
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"add_event"));
        assert!(names.contains(&"delete_event"));
        assert!(names.contains(&"list_events"));
    }

    #[tokio::test]
    async fn test_simple_initialize_has_short_instructions() {
        let (pool, _user_id, token) = setup().await;
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "clientInfo": {"name": "test", "version": "0.1"}
            }
        });
        let (_, resp) = simple_rpc_call(&pool, &token, body).await;
        let instructions = resp["result"]["instructions"].as_str().unwrap();
        assert!(
            instructions.len() < 200,
            "Simple mode instructions should be terse"
        );
        assert!(instructions.contains("list_events"));
        assert!(instructions.contains("add_event"));
    }

    #[tokio::test]
    async fn test_simple_add_creates_default_calendar_and_event() {
        let (pool, user_id, token) = setup().await;

        // No calendars exist — add should auto-create one and add the event
        let result = simple_tool_call(
            &pool,
            &token,
            "add_event",
            json!({
                "title": "Standup",
                "start": "20260301T090000Z",
                "end": "20260301T093000Z"
            }),
        )
        .await;
        assert_eq!(result["title"], "Standup");
        assert!(result["uid"].as_str().is_some());

        // Verify a default calendar was created in DB
        let cals = calendars::list_calendars_for_user(&pool, &user_id)
            .await
            .unwrap();
        assert_eq!(cals.len(), 1);
        assert_eq!(cals[0].name, "Calendar");
    }

    #[tokio::test]
    async fn test_simple_add_uses_existing_calendar() {
        let (pool, user_id, token) = setup().await;
        let cal = calendars::create_calendar(&pool, &user_id, "Work", "", "#000", "UTC")
            .await
            .unwrap();

        let result = simple_tool_call(
            &pool,
            &token,
            "add_event",
            json!({
                "title": "Lunch",
                "start": "20260301T120000Z",
                "end": "20260301T130000Z"
            }),
        )
        .await;
        assert_eq!(result["title"], "Lunch");

        // Verify event went into the existing calendar, not a new one
        let cals = calendars::list_calendars_for_user(&pool, &user_id)
            .await
            .unwrap();
        assert_eq!(cals.len(), 1);
        assert_eq!(cals[0].id, cal.id);

        let objs = crate::db::events::list_objects(&pool, &cal.id)
            .await
            .unwrap();
        assert_eq!(objs.len(), 1);
        assert_eq!(objs[0].summary.as_deref(), Some("Lunch"));
    }

    #[tokio::test]
    async fn test_simple_add_and_delete_event() {
        let (pool, user_id, token) = setup().await;
        calendars::create_calendar(&pool, &user_id, "Work", "", "#000", "UTC")
            .await
            .unwrap();

        // Add an event
        let result = simple_tool_call(
            &pool,
            &token,
            "add_event",
            json!({
                "title": "Meeting",
                "start": "20260301T090000Z",
                "end": "20260301T100000Z"
            }),
        )
        .await;
        let uid = result["uid"].as_str().unwrap().to_string();

        // Delete the event
        let result = simple_tool_call(
            &pool,
            &token,
            "delete_event",
            json!({
                "event_uid": uid
            }),
        )
        .await;
        assert_eq!(result["deleted"], true);

        // Verify event gone from DB
        let cals = calendars::list_calendars_for_user(&pool, &user_id)
            .await
            .unwrap();
        let obj = crate::db::events::get_object_by_uid(&pool, &cals[0].id, &uid)
            .await
            .unwrap();
        assert!(obj.is_none());
    }

    #[tokio::test]
    async fn test_simple_list_all_events() {
        let (pool, user_id, token) = setup().await;
        let cal = calendars::create_calendar(&pool, &user_id, "Work", "", "#000", "UTC")
            .await
            .unwrap();

        simple_tool_call(
            &pool,
            &token,
            "add_event",
            json!({
                "title": "Morning",
                "start": "20260301T090000Z",
                "end": "20260301T100000Z"
            }),
        )
        .await;
        simple_tool_call(
            &pool,
            &token,
            "add_event",
            json!({
                "title": "Afternoon",
                "start": "20260301T140000Z",
                "end": "20260301T150000Z"
            }),
        )
        .await;

        // List all events (no time range)
        let result = simple_tool_call(&pool, &token, "list_events", json!({})).await;
        assert_eq!(result["count"], 2);
        assert_eq!(result["events"].as_array().unwrap().len(), 2);

        // Verify these are from the same calendar used by CalDAV
        let objs = crate::db::events::list_objects(&pool, &cal.id)
            .await
            .unwrap();
        assert_eq!(objs.len(), 2);
    }

    #[tokio::test]
    async fn test_simple_list_with_time_range() {
        let (pool, user_id, token) = setup().await;
        calendars::create_calendar(&pool, &user_id, "Work", "", "#000", "UTC")
            .await
            .unwrap();

        simple_tool_call(
            &pool,
            &token,
            "add_event",
            json!({
                "title": "March Event",
                "start": "20260301T090000Z",
                "end": "20260301T100000Z"
            }),
        )
        .await;
        simple_tool_call(
            &pool,
            &token,
            "add_event",
            json!({
                "title": "April Event",
                "start": "20260401T090000Z",
                "end": "20260401T100000Z"
            }),
        )
        .await;

        // Filter to March only
        let result = simple_tool_call(
            &pool,
            &token,
            "list_events",
            json!({
                "start": "20260301T000000Z",
                "end": "20260331T235959Z"
            }),
        )
        .await;
        assert_eq!(result["count"], 1);
        assert_eq!(result["events"][0]["summary"], "March Event");
    }

    #[tokio::test]
    async fn test_simple_mcp_event_visible_to_caldav_db() {
        // Verify that events created via simple MCP tools are in the same
        // DB tables that CalDAV reads from — proving sync between the two.
        let (pool, user_id, token) = setup().await;
        let cal = calendars::create_calendar(&pool, &user_id, "Shared", "", "#000", "UTC")
            .await
            .unwrap();

        let result = simple_tool_call(
            &pool,
            &token,
            "add_event",
            json!({
                "title": "MCP Created",
                "start": "20260315T100000Z",
                "end": "20260315T110000Z"
            }),
        )
        .await;
        let uid = result["uid"].as_str().unwrap();

        // Query the same DB function that CalDAV REPORT uses
        let obj = crate::db::events::get_object_by_uid(&pool, &cal.id, uid)
            .await
            .unwrap()
            .expect("Event should exist in CalDAV-accessible DB");
        assert_eq!(obj.summary.as_deref(), Some("MCP Created"));
        assert!(
            obj.ical_data.contains("VEVENT"),
            "Should have valid iCal data"
        );
    }

    #[tokio::test]
    async fn test_simple_unknown_tool() {
        let (pool, _user_id, token) = setup().await;
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": "create_event", "arguments": {}}
        });
        let (status, resp) = simple_rpc_call(&pool, &token, body).await;
        assert_eq!(status, StatusCode::OK);
        // Full-mode tool names should not work in simple mode
        assert_eq!(resp["result"]["isError"], true);
    }
}
