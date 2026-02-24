use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use sqlx::SqlitePool;

use super::xml::multistatus::{MultistatusBuilder, PropContent, PropValue};
use super::xml::{APPLE_NS, CALDAV_NS, DAV_NS};
use crate::db::calendars;
use crate::db::models::User;

/// Handle PROPPATCH for a calendar collection.
/// Supports updating displayname, calendar-description, and calendar-color.
pub async fn handle_proppatch(
    State(pool): State<SqlitePool>,
    Path((_username, calendar_id)): Path<(String, String)>,
    request: Request<Body>,
) -> Response {
    let user = request.extensions().get::<User>().unwrap().clone();
    let body = axum::body::to_bytes(request.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();

    let calendar = match calendars::get_calendar_by_id(&pool, &calendar_id).await {
        Ok(Some(cal)) => cal,
        _ => {
            return (StatusCode::NOT_FOUND, "Calendar not found").into_response();
        }
    };

    // Parse the PROPPATCH body for set/remove operations
    let body_str = String::from_utf8_lossy(&body);
    let name = extract_prop_value(&body_str, "displayname");
    let description = extract_prop_value(&body_str, "calendar-description");
    let color = extract_prop_value(&body_str, "calendar-color");

    match calendars::update_calendar(
        &pool,
        &calendar.id,
        name.as_deref(),
        description.as_deref(),
        color.as_deref(),
        None,
    )
    .await
    {
        Ok(_) => {
            let href = format!("/caldav/users/{}/{}/", user.username, calendar.id);
            let mut builder = MultistatusBuilder::new();

            let mut found = Vec::new();
            if name.is_some() {
                found.push(PropValue {
                    name: "displayname".to_string(),
                    namespace: DAV_NS.to_string(),
                    value: PropContent::Empty,
                });
            }
            if description.is_some() {
                found.push(PropValue {
                    name: "calendar-description".to_string(),
                    namespace: CALDAV_NS.to_string(),
                    value: PropContent::Empty,
                });
            }
            if color.is_some() {
                found.push(PropValue {
                    name: "calendar-color".to_string(),
                    namespace: APPLE_NS.to_string(),
                    value: PropContent::Empty,
                });
            }

            builder.add_response(&href, found, vec![]);

            Response::builder()
                .status(StatusCode::MULTI_STATUS)
                .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
                .body(Body::from(builder.build()))
                .unwrap()
        }
        Err(e) => {
            tracing::error!("Failed to update calendar properties: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to update properties").into_response()
        }
    }
}

/// Simple extraction of a property value from PROPPATCH XML.
fn extract_prop_value(xml: &str, local_name: &str) -> Option<String> {
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
            Ok(Event::End(_)) if in_target => {
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
