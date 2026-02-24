use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use sqlx::SqlitePool;

use super::propfind::multistatus_response;
use super::xml::multistatus::MultistatusBuilder;
use super::xml::{parse, properties};
use crate::db::models::User;
use crate::db::{calendars, events};

/// Handle REPORT for a calendar collection: /caldav/users/{username}/{calendar_id}/
pub async fn handle_report(
    State(pool): State<SqlitePool>,
    Path((_username, calendar_id)): Path<(String, String)>,
    request: Request<Body>,
) -> Response {
    let user = request.extensions().get::<User>().unwrap().clone();
    let body = axum::body::to_bytes(request.into_body(), 256 * 1024)
        .await
        .unwrap_or_default();

    let report = match parse::parse_report(&body) {
        Some(r) => r,
        None => {
            return (StatusCode::BAD_REQUEST, "Invalid REPORT body").into_response();
        }
    };

    match report {
        parse::ReportRequest::CalendarMultiget { props, hrefs } => {
            handle_multiget(&pool, &user, &calendar_id, &props, &hrefs).await
        }
        parse::ReportRequest::CalendarQuery { props, time_range } => {
            handle_query(&pool, &user, &calendar_id, &props, time_range.as_ref()).await
        }
        parse::ReportRequest::SyncCollection { props, sync_token } => {
            handle_sync(&pool, &user, &calendar_id, &props, &sync_token).await
        }
    }
}

/// Handle calendar-multiget REPORT: fetch specific events by href.
async fn handle_multiget(
    pool: &SqlitePool,
    user: &User,
    calendar_id: &str,
    _props: &[parse::PropRequest],
    hrefs: &[String],
) -> Response {
    let mut builder = MultistatusBuilder::new();

    // Extract UIDs from hrefs
    let uids: Vec<String> = hrefs
        .iter()
        .filter_map(|href| {
            href.rsplit('/')
                .next()
                .and_then(|f| f.strip_suffix(".ics"))
                .map(|s| s.to_string())
        })
        .collect();

    let objects = events::get_objects_by_uids(pool, calendar_id, &uids)
        .await
        .unwrap_or_default();

    for obj in &objects {
        let href = properties::calendar_object_href(&user.username, calendar_id, &obj.uid);
        builder.add_response(
            &href,
            properties::calendar_object_props(&user.username, calendar_id, obj, true),
            vec![],
        );
    }

    multistatus_response(builder.build())
}

/// Handle calendar-query REPORT: fetch events matching a filter (time-range).
async fn handle_query(
    pool: &SqlitePool,
    user: &User,
    calendar_id: &str,
    _props: &[parse::PropRequest],
    time_range: Option<&(String, String)>,
) -> Response {
    let mut builder = MultistatusBuilder::new();

    let objects = match time_range {
        Some((start, end)) => events::list_objects_in_range(pool, calendar_id, start, end)
            .await
            .unwrap_or_default(),
        None => events::list_objects(pool, calendar_id)
            .await
            .unwrap_or_default(),
    };

    for obj in &objects {
        let href = properties::calendar_object_href(&user.username, calendar_id, &obj.uid);
        builder.add_response(
            &href,
            properties::calendar_object_props(&user.username, calendar_id, obj, true),
            vec![],
        );
    }

    multistatus_response(builder.build())
}

/// Handle sync-collection REPORT (RFC 6578): return changes since a sync token.
async fn handle_sync(
    pool: &SqlitePool,
    user: &User,
    calendar_id: &str,
    _props: &[parse::PropRequest],
    sync_token: &str,
) -> Response {
    let calendar = match calendars::get_calendar_by_id(pool, calendar_id).await {
        Ok(Some(cal)) => cal,
        _ => {
            return (StatusCode::NOT_FOUND, "Calendar not found").into_response();
        }
    };

    let mut builder = MultistatusBuilder::new();

    if sync_token.is_empty() {
        // Initial sync: return all objects
        let objects = events::list_objects(pool, calendar_id)
            .await
            .unwrap_or_default();

        for obj in &objects {
            let href = properties::calendar_object_href(&user.username, calendar_id, &obj.uid);
            builder.add_response(
                &href,
                properties::calendar_object_props(&user.username, calendar_id, obj, false),
                vec![],
            );
        }
    } else {
        // Delta sync: return changes since the given token
        let changes = events::get_sync_changes_since(pool, calendar_id, sync_token)
            .await
            .unwrap_or_default();

        for change in &changes {
            let href =
                properties::calendar_object_href(&user.username, calendar_id, &change.object_uid);

            if change.change_type == "deleted" {
                // For deletions, return a 404 status for that href
                builder.add_response(&href, vec![], vec![]);
            } else {
                // For created/modified, return the current object
                if let Ok(Some(obj)) =
                    events::get_object_by_uid(pool, calendar_id, &change.object_uid).await
                {
                    builder.add_response(
                        &href,
                        properties::calendar_object_props(
                            &user.username,
                            calendar_id,
                            &obj,
                            false,
                        ),
                        vec![],
                    );
                }
            }
        }
    }

    // Include the current sync token
    builder.add_sync_token(&calendar.sync_token);

    multistatus_response(builder.build())
}
