use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

/// Handle GET/PROPFIND /.well-known/caldav
/// Apple Calendar hits this first to discover the CalDAV service root.
/// Returns 301 redirect to /caldav/
pub async fn handle_well_known() -> Response {
    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::MOVED_PERMANENTLY;
    response
        .headers_mut()
        .insert(header::LOCATION, "/caldav/".parse().unwrap());
    response
}

/// Handle OPTIONS requests at any CalDAV path.
/// Returns DAV compliance headers that Apple Calendar requires.
pub async fn handle_options() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            ("DAV", "1, 2, 3, calendar-access, calendar-schedule"),
            (
                "Allow",
                "OPTIONS, GET, HEAD, PUT, DELETE, PROPFIND, PROPPATCH, REPORT, MKCALENDAR",
            ),
        ],
    )
}
