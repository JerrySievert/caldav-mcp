use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sqlx::SqlitePool;

use super::HrefContext;
use super::encode_email_for_path;
use super::xml::multistatus::MultistatusBuilder;
use super::xml::parse::{self, PropfindRequest};
use super::xml::properties;
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
    let propfind = parse::parse_propfind(&body);

    let mut builder = MultistatusBuilder::new();

    // The calendar home itself
    let (found, not_found) =
        properties::filter_props(&propfind, properties::calendar_home_props(&user.username));
    builder.add_response(
        &format!("/caldav/users/{}/", user.username),
        found,
        not_found,
    );

    // If Depth:1, list all accessible calendars
    if depth >= 1 {
        let cals = calendars::list_calendars_for_user(&pool, &user.id)
            .await
            .unwrap_or_default();

        for cal in &cals {
            let href = properties::calendar_href(&user.username, &cal.id);
            let (found, not_found) = properties::filter_props(
                &propfind,
                properties::calendar_props(&user.username, cal),
            );
            builder.add_response(&href, found, not_found);
        }
    }

    multistatus_response(builder.build())
}

/// Handle PROPFIND for a calendar collection: /caldav/users/{username}/{calendar_id}/
/// or /calendar/dav/{email}/user/{calendar_id}/
/// With Depth:1, also lists all calendar objects.
pub async fn handle_calendar(
    State(pool): State<SqlitePool>,
    Path((_username, calendar_id)): Path<(String, String)>,
    request: Request<Body>,
) -> Response {
    let user = request.extensions().get::<User>().unwrap().clone();
    let href_ctx = request.extensions().get::<HrefContext>().cloned();
    let depth = get_depth(&request);
    let body = axum::body::to_bytes(request.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    let propfind = parse::parse_propfind(&body);

    // Find the calendar
    let calendar = match calendars::get_calendar_by_id(&pool, &calendar_id).await {
        Ok(Some(cal)) => cal,
        _ => {
            return (StatusCode::NOT_FOUND, "Calendar not found").into_response();
        }
    };

    let mut builder = MultistatusBuilder::new();

    // Use context-aware hrefs if available, otherwise default to username-based
    let ctx = href_ctx.unwrap_or(HrefContext {
        email: None,
        username: user.username.clone(),
    });

    // The calendar collection itself
    let href = properties::calendar_href_for_context(&ctx, &calendar.id);
    let (found, not_found) = properties::filter_props(
        &propfind,
        properties::calendar_props_for_context(&ctx, &calendar),
    );
    builder.add_response(&href, found, not_found);

    // If Depth:1, list all calendar objects
    if depth >= 1 {
        let objects = events::list_objects(&pool, &calendar.id)
            .await
            .unwrap_or_default();

        for obj in &objects {
            let obj_href =
                properties::calendar_object_href_for_context(&ctx, &calendar.id, &obj.uid);
            let (found, not_found) = properties::filter_props(
                &propfind,
                properties::calendar_object_props(&user.username, &calendar.id, obj, false),
            );
            builder.add_response(&obj_href, found, not_found);
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
        .map(|v| match v {
            "1" => 1,
            _ => 0, // "0", "infinity" (capped at 1 for safety), and unknown all map to 0
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
    email: &str,
    propfind: &PropfindRequest,
) -> Response {
    let ctx = HrefContext {
        email: Some(encode_email_for_path(email)),
        username: user.username.clone(),
    };
    let mut builder = MultistatusBuilder::new();

    // The email home itself — advertise principal + calendar-home-set pointing
    // back to this same URL, so dataaccessd knows it's already at the right place.
    let (found, not_found) = properties::filter_props(
        propfind,
        properties::email_home_props(&user.username, email, &request_path),
    );
    builder.add_response(&request_path, found, not_found);

    // If Depth:1, include all accessible calendars with email-based hrefs
    // so dataaccessd can access them under the email path.
    if depth >= 1 {
        let cals = calendars::list_calendars_for_user(&pool, &user.id)
            .await
            .unwrap_or_default();

        for cal in &cals {
            let href = properties::calendar_href_for_context(&ctx, &cal.id);
            let (found, not_found) = properties::filter_props(
                propfind,
                properties::calendar_props_for_context(&ctx, cal),
            );
            builder.add_response(&href, found, not_found);
        }
    }

    multistatus_response(builder.build())
}

/// Build a 207 Multi-Status response with XML body.
pub fn multistatus_response(xml: Vec<u8>) -> Response {
    Response::builder()
        .status(StatusCode::MULTI_STATUS)
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .header("DAV", "1, 2, 3, calendar-access, calendar-schedule")
        .body(Body::from(xml))
        .unwrap()
}
