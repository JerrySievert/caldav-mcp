use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sqlx::SqlitePool;

use crate::db::events;
use crate::ical::parser;

/// Handle PUT for a calendar object: /caldav/users/{username}/{calendar_id}/{uid}.ics
/// Creates or updates the event.
pub async fn handle_put(
    State(pool): State<SqlitePool>,
    Path((_username, calendar_id, filename)): Path<(String, String, String)>,
    request: Request<Body>,
) -> Response {
    let uid_from_url = filename.trim_end_matches(".ics").to_string();

    // Check If-Match for conditional updates
    let if_match = request
        .headers()
        .get(header::IF_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let body = match axum::body::to_bytes(request.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Request body too large").into_response();
        }
    };

    let ical_data = match String::from_utf8(body.to_vec()) {
        Ok(s) => s,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid UTF-8 in request body").into_response();
        }
    };

    // Extract fields from the iCalendar data
    let fields = parser::extract_fields(&ical_data);
    let uid = fields.uid.as_deref().unwrap_or(&uid_from_url);

    // If If-Match is present, verify the current ETag matches
    if let Some(expected_etag) = &if_match
        && expected_etag != "*"
    {
        match events::get_object_by_uid(&pool, &calendar_id, uid).await {
            Ok(Some(existing)) => {
                if existing.etag != *expected_etag {
                    return (StatusCode::PRECONDITION_FAILED, "ETag mismatch").into_response();
                }
            }
            Ok(None) => {
                return (StatusCode::PRECONDITION_FAILED, "Object does not exist").into_response();
            }
            Err(e) => {
                tracing::error!("Failed to check existing object: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
            }
        }
    }

    // Upsert the object
    match events::upsert_object(
        &pool,
        &calendar_id,
        uid,
        &ical_data,
        events::ObjectFields {
            component_type: &fields.component_type,
            dtstart: fields.dtstart.as_deref(),
            dtend: fields.dtend.as_deref(),
            summary: fields.summary.as_deref(),
        },
    )
    .await
    {
        Ok((obj, is_new)) => {
            let status = if is_new {
                StatusCode::CREATED
            } else {
                StatusCode::NO_CONTENT
            };
            Response::builder()
                .status(status)
                .header(header::ETAG, &obj.etag)
                .body(Body::empty())
                .unwrap()
        }
        Err(e) => {
            tracing::error!("Failed to upsert object: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save event").into_response()
        }
    }
}
