use serde_json::{json, Value};
use sqlx::SqlitePool;

use super::jsonrpc::{JsonRpcErrorResponse, JsonRpcRequest, JsonRpcResponse};
use super::session::SessionManager;
use super::tools;

/// Handle an MCP JSON-RPC request. Returns the response value to serialize.
pub async fn handle_request(
    pool: &SqlitePool,
    sessions: &SessionManager,
    user_id: &str,
    request: &JsonRpcRequest,
) -> Value {
    match request.method.as_str() {
        "initialize" => handle_initialize(sessions, user_id, request),
        "notifications/initialized" => {
            // Notification — no response needed
            Value::Null
        }
        "tools/list" => handle_tools_list(request),
        "tools/call" => handle_tools_call(pool, user_id, request).await,
        "ping" => {
            serde_json::to_value(JsonRpcResponse::success(request.id.clone(), json!({}))).unwrap()
        }
        _ => serde_json::to_value(JsonRpcErrorResponse::method_not_found(request.id.clone()))
            .unwrap(),
    }
}

/// Handle the MCP initialize request.
fn handle_initialize(
    sessions: &SessionManager,
    user_id: &str,
    request: &JsonRpcRequest,
) -> Value {
    let _session_id = sessions.create_session(user_id);

    let result = json!({
        "protocolVersion": "2025-03-26",
        "capabilities": {
            "tools": {
                "listChanged": false
            }
        },
        "serverInfo": {
            "name": "caldav-mcp-server",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "This MCP server provides tools to manage CalDAV calendars and events. Use list_calendars to see available calendars, then create_event, query_events, etc. to manage events."
    });

    serde_json::to_value(JsonRpcResponse::success(request.id.clone(), result)).unwrap()
}

/// Handle tools/list — return all tool definitions.
fn handle_tools_list(request: &JsonRpcRequest) -> Value {
    let tool_defs = tools::all_tools();
    let tools_json: Vec<Value> = tool_defs
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })
        })
        .collect();

    serde_json::to_value(JsonRpcResponse::success(
        request.id.clone(),
        json!({ "tools": tools_json }),
    ))
    .unwrap()
}

/// Handle tools/call — dispatch to the appropriate tool handler.
async fn handle_tools_call(pool: &SqlitePool, user_id: &str, request: &JsonRpcRequest) -> Value {
    let tool_name = match request.params.get("name").and_then(|v| v.as_str()) {
        Some(name) => name,
        None => {
            return serde_json::to_value(JsonRpcErrorResponse::invalid_params(
                request.id.clone(),
                "Missing 'name' in params",
            ))
            .unwrap();
        }
    };

    let arguments = request
        .params
        .get("arguments")
        .cloned()
        .unwrap_or(json!({}));

    match tools::dispatch(pool, user_id, tool_name, &arguments).await {
        Ok(result) => {
            let content = json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&result).unwrap_or_default()
                }],
                "structuredContent": result,
                "isError": false
            });
            serde_json::to_value(JsonRpcResponse::success(request.id.clone(), content)).unwrap()
        }
        Err(err) => {
            let content = json!({
                "content": [{
                    "type": "text",
                    "text": err
                }],
                "isError": true
            });
            serde_json::to_value(JsonRpcResponse::success(request.id.clone(), content)).unwrap()
        }
    }
}
