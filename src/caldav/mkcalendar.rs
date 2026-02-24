use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use sqlx::SqlitePool;

use crate::db::models::User;
use crate::db::calendars;

/// Handle MKCALENDAR request to create a new calendar.
/// Path: /caldav/users/{username}/{calendar_id}/
pub async fn handle_mkcalendar(
    State(pool): State<SqlitePool>,
    Path((username, calendar_id)): Path<(String, String)>,
    request: Request<Body>,
) -> Response {
    let user = request.extensions().get::<User>().unwrap().clone();

    // Only the authenticated user can create calendars in their own space
    if user.username != username {
        return (StatusCode::FORBIDDEN, "Cannot create calendars for another user").into_response();
    }

    // Check if calendar already exists
    if let Ok(Some(_)) = calendars::get_calendar_by_id(&pool, &calendar_id).await {
        return (StatusCode::METHOD_NOT_ALLOWED, "Calendar already exists").into_response();
    }

    // Parse the request body for calendar properties (optional)
    let body = axum::body::to_bytes(request.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();

    let name = extract_displayname(&body).unwrap_or_else(|| calendar_id.clone());
    let color = extract_calendar_color(&body).unwrap_or_else(|| "#0E61B9".to_string());

    match calendars::create_calendar(&pool, &user.id, &name, "", &color, "UTC").await {
        Ok(_cal) => (StatusCode::CREATED, "Calendar created").into_response(),
        Err(e) => {
            tracing::error!("Failed to create calendar: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create calendar").into_response()
        }
    }
}

/// Extract displayname from MKCALENDAR XML body.
fn extract_displayname(body: &[u8]) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(body);
    extract_xml_value(&text, "displayname")
}

/// Extract calendar-color from MKCALENDAR XML body.
fn extract_calendar_color(body: &[u8]) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(body);
    extract_xml_value(&text, "calendar-color")
}

/// Simple XML value extraction by local element name.
fn extract_xml_value(xml: &str, local_name: &str) -> Option<String> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut in_target = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let local = e.local_name();
                let name = String::from_utf8_lossy(local.as_ref()).to_string();
                if name == local_name {
                    in_target = true;
                }
            }
            Ok(Event::Text(ref e)) if in_target => {
                return Some(e.unescape().unwrap_or_default().to_string());
            }
            Ok(Event::End(_)) => {
                in_target = false;
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    None
}
