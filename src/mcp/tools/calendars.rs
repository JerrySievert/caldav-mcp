use serde_json::{json, Value};
use sqlx::SqlitePool;

use super::ToolDef;
use crate::db::calendars as cal_db;

pub fn tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "list_calendars",
            description: "List all calendars accessible to the authenticated user (owned + shared)",
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "get_calendar",
            description: "Get details about a specific calendar",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "calendar_id": {"type": "string", "description": "The calendar ID"}
                },
                "required": ["calendar_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "create_calendar",
            description: "Create a new calendar",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Calendar display name"},
                    "description": {"type": "string", "description": "Calendar description"},
                    "color": {"type": "string", "description": "Calendar color (hex, e.g. #FF0000)"},
                    "timezone": {"type": "string", "description": "Calendar timezone (e.g. America/New_York)"}
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "delete_calendar",
            description: "Delete a calendar and all its events",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "calendar_id": {"type": "string", "description": "The calendar ID to delete"}
                },
                "required": ["calendar_id"],
                "additionalProperties": false
            }),
        },
    ]
}

pub async fn list_calendars(
    pool: &SqlitePool,
    user_id: &str,
    _args: &Value,
) -> Result<Value, String> {
    let cals = cal_db::list_calendars_for_user(pool, user_id)
        .await
        .map_err(|e| format!("Failed to list calendars: {e}"))?;

    let result: Vec<Value> = cals
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "name": c.name,
                "description": c.description,
                "color": c.color,
                "timezone": c.timezone,
                "owner_id": c.owner_id,
            })
        })
        .collect();

    Ok(json!(result))
}

pub async fn get_calendar(
    pool: &SqlitePool,
    _user_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let calendar_id = args["calendar_id"]
        .as_str()
        .ok_or("Missing calendar_id")?;

    let cal = cal_db::get_calendar_by_id(pool, calendar_id)
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or("Calendar not found")?;

    Ok(json!({
        "id": cal.id,
        "name": cal.name,
        "description": cal.description,
        "color": cal.color,
        "timezone": cal.timezone,
        "owner_id": cal.owner_id,
        "ctag": cal.ctag,
    }))
}

pub async fn create_calendar(
    pool: &SqlitePool,
    user_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let name = args["name"].as_str().ok_or("Missing name")?;
    let description = args["description"].as_str().unwrap_or("");
    let color = args["color"].as_str().unwrap_or("#0E61B9");
    let timezone = args["timezone"].as_str().unwrap_or("UTC");

    let cal = cal_db::create_calendar(pool, user_id, name, description, color, timezone)
        .await
        .map_err(|e| format!("Failed to create calendar: {e}"))?;

    Ok(json!({
        "id": cal.id,
        "name": cal.name,
        "description": cal.description,
        "color": cal.color,
        "timezone": cal.timezone,
    }))
}

pub async fn delete_calendar_tool(
    pool: &SqlitePool,
    _user_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let calendar_id = args["calendar_id"]
        .as_str()
        .ok_or("Missing calendar_id")?;

    cal_db::delete_calendar(pool, calendar_id)
        .await
        .map_err(|e| format!("Failed to delete calendar: {e}"))?;

    Ok(json!({"deleted": true, "calendar_id": calendar_id}))
}
