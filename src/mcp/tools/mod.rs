pub mod calendars;
pub mod events;
pub mod sharing;

use serde_json::Value;
use sqlx::SqlitePool;

/// A tool definition for the MCP tools/list response.
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

/// Get all registered MCP tool definitions.
pub fn all_tools() -> Vec<ToolDef> {
    let mut tools = Vec::new();
    tools.extend(calendars::tool_defs());
    tools.extend(events::tool_defs());
    tools.extend(sharing::tool_defs());
    tools
}

/// Dispatch a tools/call request to the appropriate handler.
pub async fn dispatch(
    pool: &SqlitePool,
    user_id: &str,
    tool_name: &str,
    arguments: &Value,
) -> Result<Value, String> {
    match tool_name {
        "list_calendars" => calendars::list_calendars(pool, user_id, arguments).await,
        "get_calendar" => calendars::get_calendar(pool, user_id, arguments).await,
        "create_calendar" => calendars::create_calendar(pool, user_id, arguments).await,
        "delete_calendar" => calendars::delete_calendar_tool(pool, user_id, arguments).await,
        "create_event" => events::create_event(pool, user_id, arguments).await,
        "get_event" => events::get_event(pool, user_id, arguments).await,
        "update_event" => events::update_event(pool, user_id, arguments).await,
        "delete_event" => events::delete_event(pool, user_id, arguments).await,
        "query_events" => events::query_events(pool, user_id, arguments).await,
        "share_calendar" => sharing::share_calendar(pool, user_id, arguments).await,
        "unshare_calendar" => sharing::unshare_calendar(pool, user_id, arguments).await,
        "list_shared_calendars" => sharing::list_shared_calendars(pool, user_id, arguments).await,
        _ => Err(format!("Unknown tool: {tool_name}")),
    }
}
