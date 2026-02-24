mod auth;
mod delete;
mod get;
mod mkcalendar;
pub mod propfind;
mod proppatch;
mod put;
mod report;
mod wellknown;
pub mod xml;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;

/// Build the CalDAV router. Mounted on the CalDAV port.
pub fn router(pool: SqlitePool) -> Router {
    let caldav_routes = Router::new()
        // Calendar object: /caldav/users/{username}/{calendar_id}/{uid}.ics
        .route(
            "/caldav/users/{username}/{calendar_id}/{filename}",
            any(handle_object),
        )
        // Calendar collection: /caldav/users/{username}/{calendar_id}/
        .route(
            "/caldav/users/{username}/{calendar_id}/",
            any(handle_calendar_collection),
        )
        .route(
            "/caldav/users/{username}/{calendar_id}",
            any(handle_calendar_collection),
        )
        // Calendar home: /caldav/users/{username}/
        .route("/caldav/users/{username}/", any(handle_calendar_home))
        .route("/caldav/users/{username}", any(handle_calendar_home))
        // User principal: /caldav/principals/{username}/
        .route(
            "/caldav/principals/{username}/",
            any(handle_principal),
        )
        .route(
            "/caldav/principals/{username}",
            any(handle_principal),
        )
        // CalDAV root
        .route("/caldav/", any(handle_root))
        .route("/caldav", any(handle_root))
        .layer(middleware::from_fn_with_state(
            pool.clone(),
            auth::require_auth,
        ))
        .with_state(pool.clone());

    // Well-known doesn't require auth (it just redirects)
    Router::new()
        .route(
            "/.well-known/caldav",
            any(wellknown::handle_well_known),
        )
        .merge(caldav_routes)
        .layer(TraceLayer::new_for_http())
        .with_state(pool)
}

/// Dispatch requests at the CalDAV root based on method.
async fn handle_root(
    state: State<SqlitePool>,
    request: Request<Body>,
) -> Response {
    match request.method().as_str() {
        "PROPFIND" => propfind::handle_root(state, request).await,
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
    }
}

/// Dispatch requests at a user principal based on method.
async fn handle_principal(
    state: State<SqlitePool>,
    path: Path<String>,
    request: Request<Body>,
) -> Response {
    match request.method().as_str() {
        "PROPFIND" => propfind::handle_principal(state, path, request).await,
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
    }
}

/// Dispatch requests at a calendar home based on method.
async fn handle_calendar_home(
    state: State<SqlitePool>,
    path: Path<String>,
    request: Request<Body>,
) -> Response {
    match request.method().as_str() {
        "PROPFIND" => propfind::handle_calendar_home(state, path, request).await,
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
    }
}

/// Dispatch requests at a calendar collection based on method.
async fn handle_calendar_collection(
    state: State<SqlitePool>,
    path: Path<(String, String)>,
    request: Request<Body>,
) -> Response {
    match request.method().as_str() {
        "PROPFIND" => propfind::handle_calendar(state, path, request).await,
        "REPORT" => report::handle_report(state, path, request).await,
        "MKCALENDAR" => mkcalendar::handle_mkcalendar(state, path, request).await,
        "PROPPATCH" => proppatch::handle_proppatch(state, path, request).await,
        "DELETE" => delete::handle_delete_calendar(state, path).await,
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
    }
}

/// Dispatch requests at a calendar object based on method.
async fn handle_object(
    state: State<SqlitePool>,
    path: Path<(String, String, String)>,
    request: Request<Body>,
) -> Response {
    match request.method().as_str() {
        "GET" => get::handle_get(state, path).await,
        "PUT" => put::handle_put(state, path, request).await,
        "DELETE" => delete::handle_delete_object(state, path).await,
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
    }
}
