use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

/// Handle any method on /.well-known/caldav
/// Apple Calendar hits this first to discover the CalDAV service root.
/// OPTIONS returns DAV headers; everything else returns 301 redirect to /caldav/.
///
/// Note: Apple Calendar's accountsd process does discovery without auth.
/// It expects a redirect here, then authenticates at the destination.
/// We must NOT require auth on this endpoint.
pub async fn handle_well_known(request: axum::extract::Request) -> Response {
    tracing::info!(
        method = %request.method(),
        uri = %request.uri(),
        has_auth = request.headers().get(axum::http::header::AUTHORIZATION).is_some(),
        user_agent = ?request.headers().get("user-agent").and_then(|v| v.to_str().ok()),
        "handle_well_known"
    );
    if request.method().as_str() == "OPTIONS" {
        return Response::builder()
            .status(StatusCode::OK)
            .header("DAV", "1, 2, 3, calendar-access, calendar-schedule")
            .header(
                "Allow",
                "OPTIONS, GET, HEAD, PUT, DELETE, PROPFIND, PROPPATCH, REPORT, MKCALENDAR",
            )
            .body(axum::body::Body::empty())
            .unwrap();
    }

    // Use 301 redirect. Apple Calendar's accountsd follows redirects for
    // PROPFIND during discovery and will authenticate at the destination.
    Response::builder()
        .status(StatusCode::MOVED_PERMANENTLY)
        .header(header::LOCATION, "/caldav/")
        .header("DAV", "1, 2, 3, calendar-access, calendar-schedule")
        .body(axum::body::Body::empty())
        .unwrap()
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
