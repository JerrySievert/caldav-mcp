use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::ToolDef;
use crate::db::models::Permission;
use crate::db::{shares, users};

/// Return the MCP tool definitions for calendar sharing operations.
pub fn tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "share_calendar",
            description: "Share a calendar with another user",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "calendar_id": {"type": "string", "description": "The calendar ID to share"},
                    "username": {"type": "string", "description": "Username of the user to share with"},
                    "permission": {"type": "string", "enum": ["read", "read-write"], "description": "Access level to grant"}
                },
                "required": ["calendar_id", "username", "permission"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "unshare_calendar",
            description: "Revoke a user's access to a shared calendar",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "calendar_id": {"type": "string", "description": "The calendar ID"},
                    "username": {"type": "string", "description": "Username to revoke access from"}
                },
                "required": ["calendar_id", "username"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "list_shared_calendars",
            description: "List calendars shared with the authenticated user",
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
    ]
}

/// Share a calendar with another user, granting the specified access level.
pub async fn share_calendar(
    pool: &SqlitePool,
    _user_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let calendar_id = args["calendar_id"].as_str().ok_or("Missing calendar_id")?;
    let username = args["username"].as_str().ok_or("Missing username")?;
    let permission_str = args["permission"].as_str().ok_or("Missing permission")?;

    let permission =
        Permission::from_str_value(permission_str).ok_or("Invalid permission value")?;

    let target_user = users::get_user_by_username(pool, username)
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or_else(|| format!("User '{username}' not found"))?;

    let share = shares::share_calendar(pool, calendar_id, &target_user.id, permission)
        .await
        .map_err(|e| format!("Failed to share calendar: {e}"))?;

    Ok(json!({
        "calendar_id": share.calendar_id,
        "shared_with": username,
        "permission": share.permission,
    }))
}

/// Revoke a user's access to a shared calendar.
pub async fn unshare_calendar(
    pool: &SqlitePool,
    _user_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let calendar_id = args["calendar_id"].as_str().ok_or("Missing calendar_id")?;
    let username = args["username"].as_str().ok_or("Missing username")?;

    let target_user = users::get_user_by_username(pool, username)
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or_else(|| format!("User '{username}' not found"))?;

    shares::unshare_calendar(pool, calendar_id, &target_user.id)
        .await
        .map_err(|e| format!("Failed to unshare calendar: {e}"))?;

    Ok(json!({"unshared": true, "calendar_id": calendar_id, "username": username}))
}

/// List all calendars that have been shared with the authenticated user.
pub async fn list_shared_calendars(
    pool: &SqlitePool,
    user_id: &str,
    _args: &Value,
) -> Result<Value, String> {
    let shared = shares::list_shared_calendars(pool, user_id)
        .await
        .map_err(|e| format!("Database error: {e}"))?;

    let result: Vec<Value> = shared
        .iter()
        .map(|(cal, perm)| {
            json!({
                "id": cal.id,
                "name": cal.name,
                "owner_id": cal.owner_id,
                "permission": perm.as_str(),
                "color": cal.color,
            })
        })
        .collect();

    Ok(json!({ "shared_calendars": result }))
}
