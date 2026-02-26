use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::ToolDef;
use crate::db::calendars as cal_db;
use crate::db::events as event_db;
use crate::ical::builder;

/// Simplified tool definitions for local LLMs — 3 terse tools.
/// Calendar management is hidden; all tools auto-resolve to the user's calendar.
pub fn tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "add_event",
            description: "Add a calendar event.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "Event title"},
                    "start": {"type": "string", "description": "Local start time in iCal format, e.g. 20260301T090000 (no Z suffix when timezone is provided)"},
                    "end": {"type": "string", "description": "Local end time in iCal format, e.g. 20260301T100000"},
                    "timezone": {"type": "string", "description": "IANA timezone name, e.g. America/Los_Angeles. Required for local time; omit only for explicit UTC (append Z to start/end)."},
                    "description": {"type": "string"},
                    "location": {"type": "string"}
                },
                "required": ["title", "start", "end"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "delete_event",
            description: "Delete a calendar event by its UID.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "event_uid": {"type": "string", "description": "Event UID to delete"}
                },
                "required": ["event_uid"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "list_events",
            description: "List upcoming calendar events. Optionally filter by time range.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "start": {"type": "string", "description": "Range start, e.g. 20260301T000000Z"},
                    "end": {"type": "string", "description": "Range end, e.g. 20260331T235959Z"},
                    "limit": {"type": "integer", "description": "Max results (default 50)", "minimum": 1, "maximum": 500}
                },
                "additionalProperties": false
            }),
        },
    ]
}

/// Dispatch a simple-mode tool call.
pub async fn dispatch(
    pool: &SqlitePool,
    user_id: &str,
    tool_name: &str,
    args: &Value,
) -> Result<Value, String> {
    match tool_name {
        "add_event" => handle_add(pool, user_id, args).await,
        "delete_event" => handle_delete(pool, user_id, args).await,
        "list_events" => handle_list(pool, user_id, args).await,
        _ => Err(format!("Unknown tool: {tool_name}")),
    }
}

/// Get or create the user's default calendar.
/// Returns the first calendar owned by/shared with the user. Creates one if none exist.
async fn resolve_calendar(pool: &SqlitePool, user_id: &str) -> Result<String, String> {
    let cals = cal_db::list_calendars_for_user(pool, user_id)
        .await
        .map_err(|e| format!("Failed to list calendars: {e}"))?;

    if let Some(cal) = cals.first() {
        return Ok(cal.id.clone());
    }

    // Create a default calendar
    let cal = cal_db::create_calendar(pool, user_id, "Calendar", "", "#0E61B9", "UTC")
        .await
        .map_err(|e| format!("Failed to create default calendar: {e}"))?;
    Ok(cal.id)
}

/// Add: always creates an event in the user's calendar.
async fn handle_add(pool: &SqlitePool, user_id: &str, args: &Value) -> Result<Value, String> {
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or("Missing title")?;
    let start = args
        .get("start")
        .and_then(|v| v.as_str())
        .ok_or("Missing start")?;
    let end = args
        .get("end")
        .and_then(|v| v.as_str())
        .ok_or("Missing end")?;
    let description = args.get("description").and_then(|v| v.as_str());
    let location = args.get("location").and_then(|v| v.as_str());
    let timezone = args.get("timezone").and_then(|v| v.as_str());

    let calendar_id = resolve_calendar(pool, user_id).await?;

    let uid = builder::generate_uid();
    let ical_data = builder::build_vevent(&uid, title, start, end, description, location, timezone);

    let (obj, _) = event_db::upsert_object(
        pool,
        &calendar_id,
        &uid,
        &ical_data,
        event_db::ObjectFields {
            component_type: "VEVENT",
            dtstart: Some(start),
            dtend: Some(end),
            summary: Some(title),
        },
    )
    .await
    .map_err(|e| format!("Failed to create event: {e}"))?;

    Ok(json!({
        "uid": obj.uid,
        "title": title,
        "start": start,
        "end": end,
    }))
}

/// Delete: removes an event by UID from the user's calendar.
async fn handle_delete(pool: &SqlitePool, user_id: &str, args: &Value) -> Result<Value, String> {
    let event_uid = args
        .get("event_uid")
        .and_then(|v| v.as_str())
        .ok_or("Missing event_uid")?;

    let calendar_id = resolve_calendar(pool, user_id).await?;

    event_db::delete_object(pool, &calendar_id, event_uid)
        .await
        .map_err(|e| format!("Failed to delete event: {e}"))?;

    Ok(json!({"deleted": true, "event_uid": event_uid}))
}

/// List: returns events from the user's calendar, optionally filtered by time range.
async fn handle_list(pool: &SqlitePool, user_id: &str, args: &Value) -> Result<Value, String> {
    let calendar_id = resolve_calendar(pool, user_id).await?;
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;

    let start = args.get("start").and_then(|v| v.as_str());
    let end = args.get("end").and_then(|v| v.as_str());

    let objects = match (start, end) {
        (Some(s), Some(e)) => event_db::list_objects_in_range(pool, &calendar_id, s, e)
            .await
            .map_err(|e| format!("Database error: {e}"))?,
        _ => event_db::list_objects(pool, &calendar_id)
            .await
            .map_err(|e| format!("Database error: {e}"))?,
    };

    let events: Vec<Value> = objects
        .iter()
        .take(limit)
        .map(|obj| {
            json!({
                "uid": obj.uid,
                "summary": obj.summary,
                "start": obj.dtstart,
                "end": obj.dtend,
            })
        })
        .collect();

    Ok(json!({
        "count": events.len(),
        "events": events,
    }))
}
