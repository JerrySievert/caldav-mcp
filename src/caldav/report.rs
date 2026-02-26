use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use sqlx::SqlitePool;

/// Percent-decode a URL path segment (e.g. `%40` → `@`).
fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(h1), Some(h2)) = (h1, h2)
                && let Ok(byte) = u8::from_str_radix(&format!("{h1}{h2}"), 16)
            {
                out.push(byte as char);
                continue;
            }
        }
        out.push(c);
    }
    out
}

use super::HrefContext;
use super::propfind::multistatus_response;
use super::xml::multistatus::MultistatusBuilder;
use super::xml::{parse, properties};
use crate::db::models::User;
use crate::db::{calendars, events};

/// Handle REPORT for a calendar collection: /caldav/users/{username}/{calendar_id}/
/// or /calendar/dav/{email}/user/{calendar_id}/
pub async fn handle_report(
    State(pool): State<SqlitePool>,
    Path((_username, calendar_id)): Path<(String, String)>,
    request: Request<Body>,
) -> Response {
    let user = request.extensions().get::<User>().unwrap().clone();
    let href_ctx = request.extensions().get::<HrefContext>().cloned();
    let body = axum::body::to_bytes(request.into_body(), 256 * 1024)
        .await
        .unwrap_or_default();

    let ctx = href_ctx.unwrap_or(HrefContext {
        email: None,
        username: user.username.clone(),
    });

    let body_str = String::from_utf8_lossy(&body).to_string();
    let report = match parse::parse_report(&body) {
        Some(r) => r,
        None => {
            tracing::warn!(calendar_id = %calendar_id, body = %body_str, "REPORT: failed to parse body");
            return (StatusCode::BAD_REQUEST, "Invalid REPORT body").into_response();
        }
    };

    let resp = match report {
        parse::ReportRequest::CalendarMultiget {
            ref props,
            ref hrefs,
        } => {
            tracing::info!(calendar_id = %calendar_id, hrefs = ?hrefs, "REPORT: calendar-multiget");
            handle_multiget(&pool, &ctx, &calendar_id, props, hrefs).await
        }
        parse::ReportRequest::CalendarQuery {
            ref props,
            ref time_range,
        } => {
            tracing::info!(calendar_id = %calendar_id, time_range = ?time_range, "REPORT: calendar-query");
            handle_query(&pool, &ctx, &calendar_id, props, time_range.as_ref()).await
        }
        parse::ReportRequest::SyncCollection {
            ref props,
            ref sync_token,
        } => {
            tracing::info!(calendar_id = %calendar_id, sync_token = %sync_token, "REPORT: sync-collection");
            handle_sync(&pool, &ctx, &calendar_id, props, sync_token).await
        }
    };

    let (parts, resp_body) = resp.into_parts();
    let resp_bytes = axum::body::to_bytes(resp_body, 512 * 1024)
        .await
        .unwrap_or_default();
    tracing::info!(
        calendar_id = %calendar_id,
        status = %parts.status,
        response_body = %String::from_utf8_lossy(&resp_bytes),
        "REPORT: response"
    );
    Response::from_parts(parts, Body::from(resp_bytes))
}

/// Handle calendar-multiget REPORT: fetch specific events by href.
async fn handle_multiget(
    pool: &SqlitePool,
    ctx: &HrefContext,
    calendar_id: &str,
    _props: &[parse::PropRequest],
    hrefs: &[String],
) -> Response {
    let mut builder = MultistatusBuilder::new();

    // Extract UIDs from hrefs, percent-decoding the filename component
    let uids: Vec<String> = hrefs
        .iter()
        .filter_map(|href| {
            href.rsplit('/')
                .next()
                .and_then(|f| f.strip_suffix(".ics"))
                .map(percent_decode)
        })
        .collect();

    let objects = events::get_objects_by_uids(pool, calendar_id, &uids)
        .await
        .unwrap_or_default();

    for obj in &objects {
        let href = properties::calendar_object_href_for_context(ctx, calendar_id, &obj.uid);
        builder.add_response(
            &href,
            properties::calendar_object_props(&ctx.username, calendar_id, obj, true),
            vec![],
        );
    }

    multistatus_response(builder.build())
}

/// Handle calendar-query REPORT: fetch events matching a filter (time-range).
async fn handle_query(
    pool: &SqlitePool,
    ctx: &HrefContext,
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
        let href = properties::calendar_object_href_for_context(ctx, calendar_id, &obj.uid);
        builder.add_response(
            &href,
            properties::calendar_object_props(&ctx.username, calendar_id, obj, true),
            vec![],
        );
    }

    multistatus_response(builder.build())
}

/// Handle sync-collection REPORT (RFC 6578): return changes since a sync token.
async fn handle_sync(
    pool: &SqlitePool,
    ctx: &HrefContext,
    calendar_id: &str,
    props: &[parse::PropRequest],
    sync_token: &str,
) -> Response {
    let calendar = match calendars::get_calendar_by_id(pool, calendar_id).await {
        Ok(Some(cal)) => cal,
        _ => {
            return (StatusCode::NOT_FOUND, "Calendar not found").into_response();
        }
    };

    // Include calendar-data if the client requested it
    let include_data = props.iter().any(|p| p.local_name == "calendar-data");

    let mut builder = MultistatusBuilder::new();

    if sync_token.is_empty() {
        // Initial sync: return all objects
        let objects = events::list_objects(pool, calendar_id)
            .await
            .unwrap_or_default();

        for obj in &objects {
            let href = properties::calendar_object_href_for_context(ctx, calendar_id, &obj.uid);
            builder.add_response(
                &href,
                properties::calendar_object_props(&ctx.username, calendar_id, obj, include_data),
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
                properties::calendar_object_href_for_context(ctx, calendar_id, &change.object_uid);

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
                            &ctx.username,
                            calendar_id,
                            &obj,
                            include_data,
                        ),
                        vec![],
                    );
                }
            }
        }
    }

    // Include the current sync token (must be a valid URI per RFC 6578)
    let token_uri = properties::ensure_sync_token_uri(&calendar.sync_token);
    builder.add_sync_token(&token_uri);

    multistatus_response(builder.build())
}
