use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sqlx::SqlitePool;

use crate::db::{calendars, events};

/// Handle DELETE for a calendar object: /caldav/users/{username}/{calendar_id}/{uid}.ics
pub async fn handle_delete_object(
    State(pool): State<SqlitePool>,
    Path((_username, calendar_id, filename)): Path<(String, String, String)>,
) -> Response {
    let uid = filename.trim_end_matches(".ics");

    match events::delete_object(&pool, &calendar_id, uid).await {
        Ok(()) => (StatusCode::NO_CONTENT, "").into_response(),
        Err(crate::error::AppError::NotFound(_)) => {
            (StatusCode::NOT_FOUND, "Object not found").into_response()
        }
        Err(e) => {
            tracing::error!("Failed to delete object: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
}

/// Handle DELETE for a calendar collection: /caldav/users/{username}/{calendar_id}/
pub async fn handle_delete_calendar(
    State(pool): State<SqlitePool>,
    Path((_username, calendar_id)): Path<(String, String)>,
) -> Response {
    match calendars::delete_calendar(&pool, &calendar_id).await {
        Ok(()) => (StatusCode::NO_CONTENT, "").into_response(),
        Err(crate::error::AppError::NotFound(_)) => {
            (StatusCode::NOT_FOUND, "Calendar not found").into_response()
        }
        Err(e) => {
            tracing::error!("Failed to delete calendar: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
}
