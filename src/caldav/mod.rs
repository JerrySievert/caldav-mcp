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
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;

/// Simple percent-decoding for URL path segments (e.g. %40 → @).
fn percent_decode_str(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                &s[i + 1..i + 3],
                16,
            ) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Build the CalDAV router. Mounted on the CalDAV port.
///
/// All routes use inline auth instead of middleware auth. Apple Calendar's
/// dataaccessd only sends credentials to URLs where it has previously
/// authenticated. Middleware-based 401s on new URLs cause sync failures
/// because dataaccessd doesn't retry with credentials.
pub fn router(pool: SqlitePool) -> Router {
    Router::new()
        .route("/.well-known/caldav", any(wellknown::handle_well_known))
        .route("/", any(handle_server_root))
        .route("/caldav/", any(handle_caldav_root))
        .route("/caldav", any(handle_caldav_root))
        .route(
            "/caldav/principals/{username}/",
            any(handle_principal_discovery),
        )
        .route(
            "/caldav/principals/{username}",
            any(handle_principal_discovery),
        )
        .route("/principals/", any(handle_fallback_discovery))
        .route("/principals/{username}/", any(handle_fallback_discovery))
        .route(
            "/calendar/dav/{email}/user/",
            any(handle_caldav_email_discovery),
        )
        .route("/caldav/users/{username}/", any(handle_calendar_home))
        .route("/caldav/users/{username}", any(handle_calendar_home))
        .route(
            "/caldav/users/{username}/{calendar_id}/",
            any(handle_calendar_collection),
        )
        .route(
            "/caldav/users/{username}/{calendar_id}",
            any(handle_calendar_collection),
        )
        .route(
            "/caldav/users/{username}/{calendar_id}/{filename}",
            any(handle_object),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(pool)
}

/// Extract the Authorization header from a request as an owned String.
fn extract_auth_header(request: &Request<Body>) -> Option<String> {
    request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned())
}

/// Inline auth helper: authenticate from an optional Authorization header value.
/// Returns 401 if the header is missing or credentials are invalid.
async fn inline_auth(
    pool: &SqlitePool,
    auth_header: Option<&str>,
) -> Result<crate::db::models::User, Response> {
    match auth_header {
        Some(h) => match auth::try_basic_auth(pool, h).await {
            Some(user) => Ok(user),
            None => Err(auth::unauthorized_response_fn()),
        },
        None => Err(auth::unauthorized_response_fn()),
    }
}

/// Auth helper that falls back to looking up the user by username from the
/// URL path when no Authorization header is present. Apple Calendar's
/// dataaccessd only sends credentials to the URL where accountsd originally
/// authenticated (the email discovery URL) and never sends them to
/// /caldav/users/{username}/* even after getting a 401.
async fn auth_or_path_user(
    pool: &SqlitePool,
    auth_header: Option<&str>,
    path_username: &str,
) -> Result<crate::db::models::User, Response> {
    // Try auth header first
    if let Some(h) = auth_header {
        if let Some(user) = auth::try_basic_auth(pool, h).await {
            return Ok(user);
        }
        return Err(auth::unauthorized_response_fn());
    }
    // No auth header: resolve user from path
    match crate::db::users::get_user_by_username(pool, path_username).await {
        Ok(Some(user)) => Ok(user),
        _ => Err(auth::unauthorized_response_fn()),
    }
}

/// Verify that a user has access to a calendar (owns it or has a share).
/// Returns false if the calendar doesn't exist or the user has no access.
async fn verify_calendar_access(
    pool: &SqlitePool,
    user: &crate::db::models::User,
    calendar_id: &str,
) -> bool {
    let accessible = crate::db::calendars::list_calendars_for_user(pool, &user.id)
        .await
        .unwrap_or_default();
    accessible.iter().any(|c| c.id == calendar_id)
}

/// Handle requests at the server root "/".
/// Returns a 207 even without auth so accountsd recognises this as a CalDAV
/// server. With auth we can include the real principal; without auth we still
/// return resourcetype and displayname.
async fn handle_server_root(
    State(pool): State<SqlitePool>,
    request: Request<Body>,
) -> Response {
    let method = request.method().clone();
    let auth_header = extract_auth_header(&request);
    tracing::info!(
        %method,
        uri = %request.uri(),
        has_auth = auth_header.is_some(),
        user_agent = ?request.headers().get("user-agent").and_then(|v| v.to_str().ok()),
        "handle_server_root"
    );
    match method.as_str() {
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        "PROPFIND" => {
            let mut builder = xml::multistatus::MultistatusBuilder::new();
            match inline_auth(&pool, auth_header.as_deref()).await {
                Ok(user) => {
                    builder.add_response(
                        "/",
                        xml::properties::root_props(&user.username),
                        vec![],
                    );
                }
                Err(_) => {
                    builder.add_response(
                        "/",
                        xml::properties::root_props_unauthenticated(),
                        vec![],
                    );
                }
            }
            propfind::multistatus_response(builder.build())
        }
        _ => Response::builder()
            .status(StatusCode::MOVED_PERMANENTLY)
            .header("Location", "/caldav/")
            .body(Body::empty())
            .unwrap(),
    }
}

/// Handle requests at the CalDAV root "/caldav/".
/// Returns a 207 even without auth so accountsd recognises this as a CalDAV
/// server and continues its discovery flow to the email URL.
async fn handle_caldav_root(
    State(pool): State<SqlitePool>,
    request: Request<Body>,
) -> Response {
    let method = request.method().clone();
    let auth_header = extract_auth_header(&request);
    tracing::info!(
        %method,
        uri = %request.uri(),
        has_auth = auth_header.is_some(),
        user_agent = ?request.headers().get("user-agent").and_then(|v| v.to_str().ok()),
        "handle_caldav_root"
    );
    match method.as_str() {
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        "PROPFIND" => {
            let mut builder = xml::multistatus::MultistatusBuilder::new();
            match inline_auth(&pool, auth_header.as_deref()).await {
                Ok(user) => {
                    builder.add_response(
                        "/caldav/",
                        xml::properties::root_props(&user.username),
                        vec![],
                    );
                }
                Err(_) => {
                    builder.add_response(
                        "/caldav/",
                        xml::properties::root_props_unauthenticated(),
                        vec![],
                    );
                }
            }
            propfind::multistatus_response(builder.build())
        }
        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
    }
}

/// Handle requests at a user principal "/caldav/principals/{username}/".
/// Returns principal info without requiring auth — accountsd and dataaccessd
/// need this to discover the calendar-home-set.
async fn handle_principal_discovery(
    State(_pool): State<SqlitePool>,
    Path(username): Path<String>,
    request: Request<Body>,
) -> Response {
    let method = request.method().clone();
    tracing::info!(
        %method,
        uri = %request.uri(),
        %username,
        "handle_principal_discovery"
    );
    match method.as_str() {
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        // Redirect all methods on the principals URL to the calendar home.
        // current-user-principal now points to /caldav/users/{username}/ directly.
        _ => Response::builder()
            .status(StatusCode::MOVED_PERMANENTLY)
            .header("Location", format!("/caldav/users/{username}/"))
            .body(Body::empty())
            .unwrap(),
    }
}

/// Handle the Apple Calendar fallback path /calendar/dav/{email}/user/.
///
/// This is the URL Apple's `dataaccessd` uses as its persistent sync home.
/// During account setup, accountsd hits this WITHOUT auth — we must return
/// a 207 with minimal discovery props so the account setup succeeds
/// (returning 401 here causes "unable to sign in").
///
/// After account setup, dataaccessd comes back WITH auth. With Depth:1 we
/// return the full calendar list so calendars appear.
///
/// Security: Without auth, only structural/capability props are returned
/// (no username, no calendar list). The same 207 is returned regardless of
/// whether the email exists, preventing email enumeration.
async fn handle_caldav_email_discovery(
    State(pool): State<SqlitePool>,
    Path(email): Path<String>,
    request: Request<Body>,
) -> Response {
    let method = request.method().clone();
    let auth_header = extract_auth_header(&request);
    let depth = propfind::get_depth_from_headers(request.headers());
    tracing::info!(
        %method,
        uri = %request.uri(),
        %email,
        depth,
        has_auth = auth_header.is_some(),
        "handle_caldav_email_discovery"
    );
    match method.as_str() {
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        "PROPFIND" => {
            let request_path = format!("/calendar/dav/{}/user/", email);

            match auth_header.as_deref() {
                Some(h) => {
                    // Auth header present: validate credentials.
                    // Return 401 if credentials are invalid (don't fall through
                    // to unauthenticated — that would silently ignore bad passwords).
                    match auth::try_basic_auth(&pool, h).await {
                        Some(user) => {
                            tracing::info!(username = %user.username, depth, "email discovery: authenticated");
                            propfind::handle_email_home(
                                State(pool),
                                user,
                                request_path,
                                depth,
                            ).await
                        }
                        None => auth::unauthorized_response_fn(),
                    }
                }
                None => {
                    // No auth header: look up user by email.
                    // dataaccessd needs the calendar list at Depth:1 to show
                    // calendars, but we use a generic displayname to avoid
                    // leaking the username.
                    let decoded_email = percent_decode_str(&email);
                    match crate::db::users::get_user_by_email(&pool, &decoded_email).await {
                        Ok(Some(user)) => {
                            tracing::info!("email discovery: unauthenticated, user found by email");
                            propfind::handle_email_home_unauthenticated(
                                State(pool),
                                user,
                                request_path,
                                depth,
                            ).await
                        }
                        _ => {
                            // Unknown email: return same structural 207 to prevent enumeration.
                            // Use Depth:0 only — no calendar list (there's no user to list for).
                            tracing::info!("email discovery: unauthenticated, no user found");
                            propfind::handle_email_discovery_unauthenticated(request_path).await
                        }
                    }
                }
            }
        }
        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
    }
}

/// Handle fallback discovery paths (/principals/, /principals/{username}/).
/// Returns a 207 even without auth (same pattern as /caldav/).
async fn handle_fallback_discovery(
    State(pool): State<SqlitePool>,
    request: Request<Body>,
) -> Response {
    let method = request.method().clone();
    let auth_header = extract_auth_header(&request);
    tracing::info!(
        %method,
        uri = %request.uri(),
        has_auth = auth_header.is_some(),
        "handle_fallback_discovery"
    );
    match method.as_str() {
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        "PROPFIND" => {
            let mut builder = xml::multistatus::MultistatusBuilder::new();
            match inline_auth(&pool, auth_header.as_deref()).await {
                Ok(user) => {
                    builder.add_response(
                        "/caldav/",
                        xml::properties::root_props(&user.username),
                        vec![],
                    );
                }
                Err(_) => {
                    builder.add_response(
                        "/caldav/",
                        xml::properties::root_props_unauthenticated(),
                        vec![],
                    );
                }
            }
            propfind::multistatus_response(builder.build())
        }
        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
    }
}

/// Dispatch requests at a calendar home based on method.
/// Uses path-based user resolution as fallback since dataaccessd does not
/// send credentials to /caldav/users/* URLs.
async fn handle_calendar_home(
    State(pool): State<SqlitePool>,
    path: Path<String>,
    request: Request<Body>,
) -> Response {
    match request.method().as_str() {
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        _ => {
            let auth_header = extract_auth_header(&request);
            let username = path.0.clone();
            match auth_or_path_user(&pool, auth_header.as_deref(), &username).await {
                Ok(user) => {
                    let mut req = request;
                    req.extensions_mut().insert(user);
                    match req.method().as_str() {
                        "PROPFIND" => propfind::handle_calendar_home(State(pool), Path(username), req).await,
                        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
                    }
                }
                Err(resp) => resp,
            }
        }
    }
}

/// Dispatch requests at a calendar collection based on method.
/// Uses path-based user resolution as fallback since dataaccessd does not
/// send credentials to /caldav/users/* URLs.
///
/// Verifies calendar ownership: the calendar must belong to (or be shared
/// with) the resolved user. This prevents cross-user data access via URL
/// manipulation.
async fn handle_calendar_collection(
    state: State<SqlitePool>,
    path: Path<(String, String)>,
    request: Request<Body>,
) -> Response {
    match request.method().as_str() {
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        _ => {
            let auth_header = extract_auth_header(&request);
            let username = (path.0).0.clone();
            let calendar_id = (path.0).1.clone();
            match auth_or_path_user(&state, auth_header.as_deref(), &username).await {
                Ok(user) => {
                    // Verify calendar ownership (skip for MKCALENDAR which creates new calendars)
                    let method_str = request.method().as_str().to_owned();
                    if method_str != "MKCALENDAR" {
                        if !verify_calendar_access(&state, &user, &calendar_id).await {
                            return (StatusCode::FORBIDDEN, "Access denied").into_response();
                        }
                    }
                    let mut req = request;
                    req.extensions_mut().insert(user);
                    match method_str.as_str() {
                        "PROPFIND" => propfind::handle_calendar(state, path, req).await,
                        "REPORT" => report::handle_report(state, path, req).await,
                        "MKCALENDAR" => mkcalendar::handle_mkcalendar(state, path, req).await,
                        "PROPPATCH" => proppatch::handle_proppatch(state, path, req).await,
                        "DELETE" => delete::handle_delete_calendar(state, path).await,
                        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
                    }
                }
                Err(resp) => resp,
            }
        }
    }
}

/// Dispatch requests at a calendar object based on method.
/// Uses path-based user resolution as fallback since dataaccessd does not
/// send credentials to /caldav/users/* URLs.
///
/// Verifies calendar ownership before granting access to objects.
async fn handle_object(
    state: State<SqlitePool>,
    path: Path<(String, String, String)>,
    request: Request<Body>,
) -> Response {
    match request.method().as_str() {
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        _ => {
            let auth_header = extract_auth_header(&request);
            let username = (path.0).0.clone();
            let calendar_id = (path.0).1.clone();
            match auth_or_path_user(&state, auth_header.as_deref(), &username).await {
                Ok(user) => {
                    // Verify calendar ownership
                    if !verify_calendar_access(&state, &user, &calendar_id).await {
                        return (StatusCode::FORBIDDEN, "Access denied").into_response();
                    }
                    let mut req = request;
                    req.extensions_mut().insert(user);
                    match req.method().as_str() {
                        "GET" => get::handle_get(state, path).await,
                        "PUT" => put::handle_put(state, path, req).await,
                        "DELETE" => delete::handle_delete_object(state, path).await,
                        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
                    }
                }
                Err(resp) => resp,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Method;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::db;
    use crate::db::users;
    use crate::db::calendars;

    /// Create a test pool with a user and calendar.
    async fn setup() -> (sqlx::SqlitePool, crate::db::models::User, crate::db::models::Calendar) {
        let pool = db::test_pool().await;
        let user = users::create_user(&pool, "alice", Some("alice@example.com"), "secret123")
            .await
            .unwrap();
        let cal = calendars::create_calendar(&pool, &user.id, "Work", "Work events", "#FF0000", "UTC")
            .await
            .unwrap();
        (pool, user, cal)
    }

    fn basic_auth_header(username: &str, password: &str) -> String {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
        format!("Basic {encoded}")
    }

    // --- Email discovery endpoint ---

    #[tokio::test]
    async fn test_email_discovery_unauthenticated_no_username_leak() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/calendar/dav/alice%40example.com/user/")
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);

        // Should NOT contain the username
        assert!(!body_str.contains(">alice<"), "Username leaked in unauthenticated response");
        // Should contain generic displayname
        assert!(body_str.contains("CalDAV Account"));
    }

    #[tokio::test]
    async fn test_email_discovery_unauthenticated_known_email_returns_calendars_no_username() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        // Known email at Depth:1 should return calendar list (dataaccessd needs it)
        // but NOT the username in the displayname
        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/calendar/dav/alice%40example.com/user/")
            .header("Depth", "1")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);

        // Calendar list IS returned (dataaccessd needs it for sync)
        assert!(body_str.contains("Work"), "Calendar list should be present for known email");
        // But username is NOT in the displayname
        assert!(body_str.contains("CalDAV Account"), "Should use generic displayname");
    }

    #[tokio::test]
    async fn test_email_discovery_unauthenticated_unknown_email_no_calendar_list() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        // Unknown email at Depth:1 should NOT return any calendar data
        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/calendar/dav/unknown%40example.com/user/")
            .header("Depth", "1")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);

        assert!(!body_str.contains("Work"), "Calendar name leaked for unknown email");
        assert!(!body_str.contains("#FF0000"), "Calendar color leaked for unknown email");
    }

    #[tokio::test]
    async fn test_email_discovery_no_enumeration() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool.clone());

        // Valid email
        let req1 = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/calendar/dav/alice%40example.com/user/")
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();
        let resp1 = app.oneshot(req1).await.unwrap();
        let status1 = resp1.status();

        // Invalid email - should get same status code
        let app2 = router(pool);
        let req2 = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/calendar/dav/nonexistent%40example.com/user/")
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();
        let resp2 = app2.oneshot(req2).await.unwrap();
        let status2 = resp2.status();

        assert_eq!(status1, status2, "Different status codes for valid/invalid emails allows enumeration");
        assert_eq!(status1, StatusCode::MULTI_STATUS);
    }

    #[tokio::test]
    async fn test_email_discovery_authenticated_returns_full_data() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/calendar/dav/alice%40example.com/user/")
            .header("Depth", "1")
            .header("Authorization", basic_auth_header("alice", "secret123"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);

        // Authenticated: should contain real username and calendar data
        assert!(body_str.contains(">alice<"), "Username should be present when authenticated");
        assert!(body_str.contains("Work"), "Calendar name should be present when authenticated");
    }

    // --- Calendar home endpoint ---

    #[tokio::test]
    async fn test_calendar_home_invalid_user_returns_401() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/caldav/users/nonexistent/")
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // --- Calendar ownership ---

    #[tokio::test]
    async fn test_calendar_access_denied_for_wrong_calendar() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/caldav/users/alice/nonexistent-calendar/")
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_calendar_access_allowed_for_own_calendar() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri(&format!("/caldav/users/alice/{}/", cal.id))
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);
    }

    #[tokio::test]
    async fn test_object_access_denied_for_wrong_calendar() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method("GET")
            .uri("/caldav/users/alice/nonexistent-calendar/test-uid.ics")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // --- Discovery endpoints ---

    #[tokio::test]
    async fn test_root_unauthenticated_no_user_leak() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/")
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);

        assert!(body_str.contains("unauthenticated"), "Should contain unauthenticated marker");
        assert!(!body_str.contains(">alice<"), "Should not leak username");
    }

    #[tokio::test]
    async fn test_caldav_root_unauthenticated_no_user_leak() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/caldav/")
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);

        assert!(body_str.contains("unauthenticated"), "Should contain unauthenticated marker");
        assert!(!body_str.contains(">alice<"), "Should not leak username");
    }

    // --- Cross-user protection ---

    #[tokio::test]
    async fn test_cross_user_calendar_access_denied() {
        let pool = db::test_pool().await;
        let alice = users::create_user(&pool, "alice", None, "pass1").await.unwrap();
        let _bob = users::create_user(&pool, "bob", None, "pass2").await.unwrap();
        let alice_cal = calendars::create_calendar(&pool, &alice.id, "Alice Cal", "", "#000", "UTC")
            .await
            .unwrap();

        let app = router(pool);

        // Bob trying to access Alice's calendar by manipulating the URL
        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri(&format!("/caldav/users/bob/{}/", alice_cal.id))
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "Bob should not access Alice's calendar");
    }

    #[tokio::test]
    async fn test_authenticated_wrong_credentials_returns_401() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/calendar/dav/alice%40example.com/user/")
            .header("Depth", "1")
            .header("Authorization", basic_auth_header("alice", "wrong-password"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
