use serde_json::{json, Value};
use sqlx::SqlitePool;

use super::ToolDef;
use crate::db::events as event_db;
use crate::ical::builder;

pub fn tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "create_event",
            description: "Create a new calendar event",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "calendar_id": {"type": "string", "description": "The target calendar ID"},
                    "title": {"type": "string", "description": "Event title/summary"},
                    "start": {"type": "string", "description": "Start time (ISO 8601 or iCal format, e.g. 20260301T090000Z)"},
                    "end": {"type": "string", "description": "End time (ISO 8601 or iCal format)"},
                    "description": {"type": "string", "description": "Event description"},
                    "location": {"type": "string", "description": "Event location"}
                },
                "required": ["calendar_id", "title", "start", "end"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "get_event",
            description: "Get a specific event by its UID",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "calendar_id": {"type": "string", "description": "The calendar ID"},
                    "event_uid": {"type": "string", "description": "The event UID"}
                },
                "required": ["calendar_id", "event_uid"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "update_event",
            description: "Update an existing event (replaces the entire event)",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "calendar_id": {"type": "string", "description": "The calendar ID"},
                    "event_uid": {"type": "string", "description": "The event UID to update"},
                    "title": {"type": "string", "description": "New event title"},
                    "start": {"type": "string", "description": "New start time"},
                    "end": {"type": "string", "description": "New end time"},
                    "description": {"type": "string", "description": "New description"},
                    "location": {"type": "string", "description": "New location"}
                },
                "required": ["calendar_id", "event_uid", "title", "start", "end"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "delete_event",
            description: "Delete a calendar event",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "calendar_id": {"type": "string", "description": "The calendar ID"},
                    "event_uid": {"type": "string", "description": "The event UID to delete"}
                },
                "required": ["calendar_id", "event_uid"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "query_events",
            description: "Query events in a calendar, optionally filtered by time range",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "calendar_id": {"type": "string", "description": "The calendar ID"},
                    "start": {"type": "string", "description": "Range start (iCal format, e.g. 20260301T000000Z)"},
                    "end": {"type": "string", "description": "Range end (iCal format)"},
                    "limit": {"type": "integer", "description": "Max events to return (default 50)", "minimum": 1, "maximum": 500}
                },
                "required": ["calendar_id"],
                "additionalProperties": false
            }),
        },
    ]
}

pub async fn create_event(
    pool: &SqlitePool,
    _user_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let calendar_id = args["calendar_id"].as_str().ok_or("Missing calendar_id")?;
    let title = args["title"].as_str().ok_or("Missing title")?;
    let start = args["start"].as_str().ok_or("Missing start")?;
    let end = args["end"].as_str().ok_or("Missing end")?;
    let description = args["description"].as_str();
    let location = args["location"].as_str();

    let uid = builder::generate_uid();
    let ical_data = builder::build_vevent(&uid, title, start, end, description, location);

    let (obj, _) = event_db::upsert_object(
        pool,
        calendar_id,
        &uid,
        &ical_data,
        "VEVENT",
        Some(start),
        Some(end),
        Some(title),
    )
    .await
    .map_err(|e| format!("Failed to create event: {e}"))?;

    Ok(json!({
        "uid": obj.uid,
        "calendar_id": calendar_id,
        "title": title,
        "start": start,
        "end": end,
        "etag": obj.etag,
    }))
}

pub async fn get_event(
    pool: &SqlitePool,
    _user_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let calendar_id = args["calendar_id"].as_str().ok_or("Missing calendar_id")?;
    let event_uid = args["event_uid"].as_str().ok_or("Missing event_uid")?;

    let obj = event_db::get_object_by_uid(pool, calendar_id, event_uid)
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or("Event not found")?;

    Ok(json!({
        "uid": obj.uid,
        "calendar_id": obj.calendar_id,
        "summary": obj.summary,
        "dtstart": obj.dtstart,
        "dtend": obj.dtend,
        "etag": obj.etag,
        "ical_data": obj.ical_data,
    }))
}

pub async fn update_event(
    pool: &SqlitePool,
    _user_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let calendar_id = args["calendar_id"].as_str().ok_or("Missing calendar_id")?;
    let event_uid = args["event_uid"].as_str().ok_or("Missing event_uid")?;
    let title = args["title"].as_str().ok_or("Missing title")?;
    let start = args["start"].as_str().ok_or("Missing start")?;
    let end = args["end"].as_str().ok_or("Missing end")?;
    let description = args["description"].as_str();
    let location = args["location"].as_str();

    // Verify the event exists
    event_db::get_object_by_uid(pool, calendar_id, event_uid)
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or("Event not found")?;

    let ical_data = builder::build_vevent(event_uid, title, start, end, description, location);

    let (obj, _) = event_db::upsert_object(
        pool,
        calendar_id,
        event_uid,
        &ical_data,
        "VEVENT",
        Some(start),
        Some(end),
        Some(title),
    )
    .await
    .map_err(|e| format!("Failed to update event: {e}"))?;

    Ok(json!({
        "uid": obj.uid,
        "calendar_id": calendar_id,
        "title": title,
        "etag": obj.etag,
        "updated": true,
    }))
}

pub async fn delete_event(
    pool: &SqlitePool,
    _user_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let calendar_id = args["calendar_id"].as_str().ok_or("Missing calendar_id")?;
    let event_uid = args["event_uid"].as_str().ok_or("Missing event_uid")?;

    event_db::delete_object(pool, calendar_id, event_uid)
        .await
        .map_err(|e| format!("Failed to delete event: {e}"))?;

    Ok(json!({"deleted": true, "event_uid": event_uid}))
}

pub async fn query_events(
    pool: &SqlitePool,
    _user_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let calendar_id = args["calendar_id"].as_str().ok_or("Missing calendar_id")?;
    let start = args["start"].as_str();
    let end = args["end"].as_str();
    let limit = args["limit"].as_u64().unwrap_or(50) as usize;

    let objects = match (start, end) {
        (Some(s), Some(e)) => event_db::list_objects_in_range(pool, calendar_id, s, e)
            .await
            .map_err(|e| format!("Database error: {e}"))?,
        _ => event_db::list_objects(pool, calendar_id)
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
                "dtstart": obj.dtstart,
                "dtend": obj.dtend,
                "etag": obj.etag,
            })
        })
        .collect();

    Ok(json!({
        "calendar_id": calendar_id,
        "count": events.len(),
        "events": events,
    }))
}
