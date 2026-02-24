use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use sqlx::SqlitePool;

use super::xml::multistatus::MultistatusBuilder;
use super::xml::{parse, properties};
use crate::db::models::User;
use crate::db::{calendars, events};

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
    get_depth_from_headers(request.headers())
}

/// Extract the Depth header value from a HeaderMap (0 or 1, default 0).
/// Public so other modules can extract depth before consuming the request.
pub fn get_depth_from_headers(headers: &axum::http::HeaderMap) -> u32 {
    headers
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

/// Handle PROPFIND for the Apple-proprietary email home URL:
/// /calendar/dav/{email}/user/
///
/// `dataaccessd` uses this URL as its persistent sync home — it probes it
/// with auth on every sync cycle. We treat it identically to the calendar
/// home at /caldav/users/{username}/: return principal + home props, and
/// with Depth:1 include the full calendar list.
///
/// The `request_path` is the URL as seen by the client (e.g.
/// "/calendar/dav/jerry%40example.com/user/") so hrefs in the 207 response
/// match what the client asked for.
pub async fn handle_email_home(
    State(pool): State<SqlitePool>,
    user: User,
    request_path: String,
    depth: u32,
) -> Response {
    let mut builder = MultistatusBuilder::new();

    // The email home itself — advertise principal + calendar-home-set pointing
    // back to this same URL, so dataaccessd knows it's already at the right place.
    builder.add_response(
        &request_path,
        properties::email_home_props(&user.username, &request_path),
        vec![],
    );

    // If Depth:1, include all accessible calendars
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

/// Handle unauthenticated PROPFIND at the email discovery URL when the
/// email matches a known user. Returns the calendar list (dataaccessd needs
/// this at Depth:1) but uses a generic displayname to avoid leaking the
/// actual username.
pub async fn handle_email_home_unauthenticated(
    State(pool): State<SqlitePool>,
    user: User,
    request_path: String,
    depth: u32,
) -> Response {
    let mut builder = MultistatusBuilder::new();

    // Use the unauthenticated props (generic displayname, no username)
    builder.add_response(
        &request_path,
        properties::email_home_props_unauthenticated(&request_path),
        vec![],
    );

    // If Depth:1, include all accessible calendars — dataaccessd needs this
    // to populate the calendar list in Apple Calendar.
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

    multistatus_response_with_auth_challenge(builder.build())
}

/// Handle unauthenticated PROPFIND at the email discovery URL when the
/// email does NOT match any user. Returns only minimal structural props.
/// Produces the same Depth:0 response shape as the known-user case to
/// prevent email enumeration.
pub async fn handle_email_discovery_unauthenticated(request_path: String) -> Response {
    let mut builder = MultistatusBuilder::new();
    builder.add_response(
        &request_path,
        properties::email_home_props_unauthenticated(&request_path),
        vec![],
    );
    // Include WWW-Authenticate header to hint that auth is available.
    // accountsd uses this to prompt for credentials.
    multistatus_response_with_auth_challenge(builder.build())
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

/// Build a 207 Multi-Status response that also includes a WWW-Authenticate
/// header, hinting the client to retry with credentials.
pub fn multistatus_response_with_auth_challenge(xml: Vec<u8>) -> Response {
    Response::builder()
        .status(StatusCode::MULTI_STATUS)
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .header("DAV", "1, 2, 3, calendar-access, calendar-schedule")
        .header(header::WWW_AUTHENTICATE, "Basic realm=\"CalDAV\"")
        .body(Body::from(xml))
        .unwrap()
}
