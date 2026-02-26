use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use sqlx::SqlitePool;

use crate::db::events;

/// Handle GET for a calendar object: /caldav/users/{username}/{calendar_id}/{uid}.ics
pub async fn handle_get(
    State(pool): State<SqlitePool>,
    Path((_username, calendar_id, filename)): Path<(String, String, String)>,
) -> Response {
    let uid = filename.trim_end_matches(".ics");

    let object = match events::get_object_by_uid(&pool, &calendar_id, uid).await {
        Ok(Some(obj)) => obj,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "Object not found").into_response();
        }
        Err(e) => {
            tracing::error!("Failed to get object: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/calendar; charset=utf-8")
        .header(header::ETAG, &object.etag)
        .body(Body::from(object.ical_data))
        .unwrap()
}
