use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use sqlx::SqlitePool;

use super::xml::multistatus::MultistatusBuilder;
use super::xml::{parse, properties};
use crate::db::models::User;
use crate::db::{calendars, events};

/// Handle PROPFIND for the CalDAV root: /caldav/
pub async fn handle_root(
    State(_pool): State<SqlitePool>,
    request: Request<Body>,
) -> Response {
    let user = request.extensions().get::<User>().unwrap().clone();
    let body = axum::body::to_bytes(request.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    let _propfind = parse::parse_propfind(&body);

    let mut builder = MultistatusBuilder::new();
    builder.add_response(
        "/caldav/",
        properties::root_props(&user.username),
        vec![],
    );

    multistatus_response(builder.build())
}

/// Handle PROPFIND for a user principal: /caldav/principals/{username}/
pub async fn handle_principal(
    State(_pool): State<SqlitePool>,
    Path(_username): Path<String>,
    request: Request<Body>,
) -> Response {
    let user = request.extensions().get::<User>().unwrap().clone();
    let body = axum::body::to_bytes(request.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    let _propfind = parse::parse_propfind(&body);

    let mut builder = MultistatusBuilder::new();
    builder.add_response(
        &format!("/caldav/principals/{}/", user.username),
        properties::principal_props(&user.username),
        vec![],
    );

    multistatus_response(builder.build())
}

/// Handle PROPFIND for calendar home: /caldav/users/{username}/
/// With Depth:1, also lists all calendars.
pub async fn handle_calendar_home(
    State(pool): State<SqlitePool>,
    Path(_username): Path<String>,
    request: Request<Body>,
) -> Response {
    let user = request.extensions().get::<User>().unwrap().clone();
    let depth = get_depth(&request);
    let body = axum::body::to_bytes(request.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    let _propfind = parse::parse_propfind(&body);

    let mut builder = MultistatusBuilder::new();

    // The calendar home itself
    builder.add_response(
        &format!("/caldav/users/{}/", user.username),
        properties::calendar_home_props(&user.username),
        vec![],
    );

    // If Depth:1, list all accessible calendars
    if depth >= 1 {
        let cals = calendars::list_calendars_for_user(&pool, &user.id)
            .await
            .unwrap_or_default();

        for cal in &cals {
            let href = properties::calendar_href(&user.username, &cal.id);
            builder.add_response(
                &href,
                properties::calendar_props(&user.username, cal),
                vec![],
            );
        }
    }

    multistatus_response(builder.build())
}

/// Handle PROPFIND for a calendar collection: /caldav/users/{username}/{calendar_id}/
/// With Depth:1, also lists all calendar objects.
pub async fn handle_calendar(
    State(pool): State<SqlitePool>,
    Path((_username, calendar_id)): Path<(String, String)>,
    request: Request<Body>,
) -> Response {
    let user = request.extensions().get::<User>().unwrap().clone();
    let depth = get_depth(&request);
    let body = axum::body::to_bytes(request.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    let _propfind = parse::parse_propfind(&body);

    // Find the calendar
    let calendar = match calendars::get_calendar_by_id(&pool, &calendar_id).await {
        Ok(Some(cal)) => cal,
        _ => {
            return (StatusCode::NOT_FOUND, "Calendar not found").into_response();
        }
    };

    let mut builder = MultistatusBuilder::new();

    // The calendar collection itself
    let href = properties::calendar_href(&user.username, &calendar.id);
    builder.add_response(
        &href,
        properties::calendar_props(&user.username, &calendar),
        vec![],
    );

    // If Depth:1, list all calendar objects
    if depth >= 1 {
        let objects = events::list_objects(&pool, &calendar.id)
            .await
            .unwrap_or_default();

        for obj in &objects {
            let obj_href =
                properties::calendar_object_href(&user.username, &calendar.id, &obj.uid);
            builder.add_response(
                &obj_href,
                properties::calendar_object_props(&user.username, &calendar.id, obj, false),
                vec![],
            );
        }
    }

    multistatus_response(builder.build())
}

/// Extract the Depth header value (0 or 1, default 0).
fn get_depth<T>(request: &Request<T>) -> u32 {
    request
        .headers()
        .get("Depth")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| match v {
            "0" => Some(0),
            "1" => Some(1),
            "infinity" => Some(1), // Treat infinity as 1 for safety
            _ => Some(0),
        })
        .unwrap_or(0)
}

/// Build a 207 Multi-Status response with XML body.
pub fn multistatus_response(xml: Vec<u8>) -> Response {
    Response::builder()
        .status(StatusCode::MULTI_STATUS)
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .header("DAV", "1, 2, 3, calendar-access")
        .body(Body::from(xml))
        .unwrap()
}
