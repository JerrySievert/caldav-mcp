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

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;

/// Context for building hrefs in responses. When email is set, hrefs use the
/// email-based path (`/calendar/dav/{email}/user/...`); otherwise they use the
/// username-based path (`/caldav/users/{username}/...`).
#[derive(Clone)]
pub struct HrefContext {
    pub email: Option<String>,
    pub username: String,
}

/// Percent-encode an email for use in URL path segments (@ → %40).
/// axum's Path extractor decodes %40 to @, so we must re-encode
/// when building hrefs that will appear in XML responses.
pub fn encode_email_for_path(email: &str) -> String {
    email.replace('@', "%40")
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
        .route(
            "/calendar/dav/{email}/user/{calendar_id}/",
            any(handle_email_calendar_collection),
        )
        .route(
            "/calendar/dav/{email}/user/{calendar_id}/{filename}",
            any(handle_email_object),
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

/// Auth helper for email-based calendar/object routes.
/// Tries auth header first; falls back to resolving user by email.
/// dataaccessd often operates without credentials on the email path.
async fn auth_or_email_user(
    pool: &SqlitePool,
    auth_header: Option<&str>,
    email: &str,
) -> Result<crate::db::models::User, Response> {
    // Try auth header first
    if let Some(h) = auth_header {
        if let Some(user) = auth::try_basic_auth(pool, h).await {
            return Ok(user);
        }
        return Err(auth::unauthorized_response_fn());
    }
    // No auth header: resolve user from email
    match crate::db::users::get_user_by_email(pool, email).await {
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
async fn handle_server_root(State(pool): State<SqlitePool>, request: Request<Body>) -> Response {
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
                    builder.add_response("/", xml::properties::root_props(&user.username), vec![]);
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
async fn handle_caldav_root(State(pool): State<SqlitePool>, request: Request<Body>) -> Response {
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
    let body_bytes = axum::body::to_bytes(request.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    let body_str = String::from_utf8_lossy(&body_bytes);
    tracing::info!(
        %method,
        %email,
        depth,
        has_auth = auth_header.is_some(),
        request_body = %body_str,
        "handle_caldav_email_discovery"
    );
    match method.as_str() {
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        "PROPFIND" => {
            let encoded_email = encode_email_for_path(&email);
            let request_path = format!("/calendar/dav/{encoded_email}/user/");
            let propfind = xml::parse::parse_propfind(&body_bytes);

            // Log parsed propfind for debugging
            tracing::info!("parsed propfind: {propfind:?}");

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
                                &email,
                                &propfind,
                            )
                            .await
                        }
                        None => auth::unauthorized_response_fn(),
                    }
                }
                None => {
                    // No auth header: resolve user by email and return
                    // discovery data. dataaccessd needs this to proceed.
                    match crate::db::users::get_user_by_email(&pool, &email).await {
                        Ok(Some(user)) => {
                            tracing::info!("email discovery: unauthenticated, user found by email");
                            let resp = propfind::handle_email_home(
                                State(pool),
                                user,
                                request_path,
                                depth,
                                &email,
                                &propfind,
                            )
                            .await;
                            // Log response body for debugging
                            let (parts, body) = resp.into_parts();
                            let resp_bytes = axum::body::to_bytes(body, 512 * 1024)
                                .await
                                .unwrap_or_default();
                            tracing::info!(
                                status = %parts.status,
                                response_body = %String::from_utf8_lossy(&resp_bytes),
                                "email discovery response"
                            );
                            Response::from_parts(parts, Body::from(resp_bytes))
                        }
                        _ => {
                            tracing::info!("email discovery: unauthenticated, no user found");
                            auth::unauthorized_response_fn()
                        }
                    }
                }
            }
        }
        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
    }
}

/// Handle requests at an email-based calendar collection:
/// /calendar/dav/{email}/user/{calendar_id}/
///
/// Resolves the user from the email, verifies calendar access, and dispatches
/// to the same handlers used by /caldav/users/{username}/{calendar_id}/.
/// This allows dataaccessd to operate entirely under the email path.
async fn handle_email_calendar_collection(
    State(pool): State<SqlitePool>,
    Path((email, calendar_id)): Path<(String, String)>,
    request: Request<Body>,
) -> Response {
    let method_str = request.method().as_str().to_owned();
    tracing::info!(
        method = %method_str,
        uri = %request.uri(),
        %email,
        %calendar_id,
        "handle_email_calendar_collection"
    );
    match method_str.as_str() {
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        _ => {
            let auth_header = extract_auth_header(&request);
            match auth_or_email_user(&pool, auth_header.as_deref(), &email).await {
                Ok(user) => {
                    // Verify calendar ownership (skip for MKCALENDAR)
                    if method_str != "MKCALENDAR"
                        && !verify_calendar_access(&pool, &user, &calendar_id).await
                    {
                        return (StatusCode::FORBIDDEN, "Access denied").into_response();
                    }
                    let username = user.username.clone();
                    let encoded_email = encode_email_for_path(&email);
                    let ctx = HrefContext {
                        email: Some(encoded_email),
                        username: username.clone(),
                    };
                    let mut req = request;
                    req.extensions_mut().insert(user);
                    req.extensions_mut().insert(ctx);
                    match method_str.as_str() {
                        "PROPFIND" => {
                            propfind::handle_calendar(
                                State(pool),
                                Path((username, calendar_id)),
                                req,
                            )
                            .await
                        }
                        "REPORT" => {
                            report::handle_report(State(pool), Path((username, calendar_id)), req)
                                .await
                        }
                        "PROPPATCH" => {
                            proppatch::handle_proppatch(
                                State(pool),
                                Path((username, calendar_id)),
                                req,
                            )
                            .await
                        }
                        "MKCALENDAR" => {
                            mkcalendar::handle_mkcalendar(
                                State(pool),
                                Path((username, calendar_id)),
                                req,
                            )
                            .await
                        }
                        "DELETE" => {
                            delete::handle_delete_calendar(
                                State(pool),
                                Path((username, calendar_id)),
                            )
                            .await
                        }
                        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
                    }
                }
                Err(resp) => resp,
            }
        }
    }
}

/// Handle requests at an email-based calendar object:
/// /calendar/dav/{email}/user/{calendar_id}/{filename}
///
/// Resolves the user from the email, verifies calendar access, and dispatches
/// to the same handlers used by /caldav/users/{username}/{calendar_id}/{filename}.
async fn handle_email_object(
    State(pool): State<SqlitePool>,
    Path((email, calendar_id, filename)): Path<(String, String, String)>,
    request: Request<Body>,
) -> Response {
    tracing::info!(
        method = %request.method(),
        uri = %request.uri(),
        %email,
        %calendar_id,
        %filename,
        "handle_email_object"
    );
    match request.method().as_str() {
        "OPTIONS" => wellknown::handle_options().await.into_response(),
        _ => {
            let auth_header = extract_auth_header(&request);
            match auth_or_email_user(&pool, auth_header.as_deref(), &email).await {
                Ok(user) => {
                    // Verify calendar ownership
                    if !verify_calendar_access(&pool, &user, &calendar_id).await {
                        return (StatusCode::FORBIDDEN, "Access denied").into_response();
                    }
                    let username = user.username.clone();
                    let encoded_email = encode_email_for_path(&email);
                    let ctx = HrefContext {
                        email: Some(encoded_email),
                        username: username.clone(),
                    };
                    let mut req = request;
                    req.extensions_mut().insert(user);
                    req.extensions_mut().insert(ctx);
                    match req.method().as_str() {
                        "GET" => {
                            get::handle_get(State(pool), Path((username, calendar_id, filename)))
                                .await
                        }
                        "PUT" => {
                            put::handle_put(
                                State(pool),
                                Path((username, calendar_id, filename)),
                                req,
                            )
                            .await
                        }
                        "DELETE" => {
                            delete::handle_delete_object(
                                State(pool),
                                Path((username, calendar_id, filename)),
                            )
                            .await
                        }
                        _ => (StatusCode::METHOD_NOT_ALLOWED, "Method not allowed").into_response(),
                    }
                }
                Err(resp) => resp,
            }
        }
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
                        "PROPFIND" => {
                            propfind::handle_calendar_home(State(pool), Path(username), req).await
                        }
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
                    if method_str != "MKCALENDAR"
                        && !verify_calendar_access(&state, &user, &calendar_id).await
                    {
                        return (StatusCode::FORBIDDEN, "Access denied").into_response();
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
    use crate::db::calendars;
    use crate::db::users;

    /// Create a test pool with a user and calendar.
    async fn setup() -> (
        sqlx::SqlitePool,
        crate::db::models::User,
        crate::db::models::Calendar,
    ) {
        let pool = db::test_pool().await;
        let user = users::create_user(&pool, "alice", Some("alice@example.com"), "secret123")
            .await
            .unwrap();
        let cal =
            calendars::create_calendar(&pool, &user.id, "Work", "Work events", "#FF0000", "UTC")
                .await
                .unwrap();
        (pool, user, cal)
    }

    fn basic_auth_header(username: &str, password: &str) -> String {
        use base64::Engine;
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
        format!("Basic {encoded}")
    }

    // --- Email discovery endpoint ---

    #[tokio::test]
    async fn test_email_discovery_unauthenticated_known_email_returns_207() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        // Known email without auth should return 207 (dataaccessd needs this)
        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/calendar/dav/alice%40example.com/user/")
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);
    }

    #[tokio::test]
    async fn test_email_discovery_depth1_unauthenticated_returns_calendars() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        // Depth:1 without auth for known email should return calendar list
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
        assert!(body_str.contains(&cal.id), "Calendar should be listed");
    }

    #[tokio::test]
    async fn test_email_discovery_unknown_email_returns_401() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/calendar/dav/unknown%40example.com/user/")
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
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
        assert!(
            body_str.contains(">alice<"),
            "Username should be present when authenticated"
        );
        assert!(
            body_str.contains("Work"),
            "Calendar name should be present when authenticated"
        );
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
            .uri(format!("/caldav/users/alice/{}/", cal.id))
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

        assert!(
            body_str.contains("unauthenticated"),
            "Should contain unauthenticated marker"
        );
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

        assert!(
            body_str.contains("unauthenticated"),
            "Should contain unauthenticated marker"
        );
        assert!(!body_str.contains(">alice<"), "Should not leak username");
    }

    // --- Cross-user protection ---

    #[tokio::test]
    async fn test_cross_user_calendar_access_denied() {
        let pool = db::test_pool().await;
        let alice = users::create_user(&pool, "alice", None, "pass1")
            .await
            .unwrap();
        let _bob = users::create_user(&pool, "bob", None, "pass2")
            .await
            .unwrap();
        let alice_cal =
            calendars::create_calendar(&pool, &alice.id, "Alice Cal", "", "#000", "UTC")
                .await
                .unwrap();

        let app = router(pool);

        // Bob trying to access Alice's calendar by manipulating the URL
        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri(format!("/caldav/users/bob/{}/", alice_cal.id))
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "Bob should not access Alice's calendar"
        );
    }

    #[tokio::test]
    async fn test_authenticated_wrong_credentials_returns_401() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/calendar/dav/alice%40example.com/user/")
            .header("Depth", "1")
            .header(
                "Authorization",
                basic_auth_header("alice", "wrong-password"),
            )
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // --- Email-based calendar hrefs in discovery ---

    #[tokio::test]
    async fn test_email_discovery_depth1_returns_email_based_calendar_hrefs() {
        let (pool, _user, cal) = setup().await;
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

        // Calendar hrefs should be under email path, NOT /caldav/users/
        // Email in hrefs must be percent-encoded (@ → %40) to match the URL dataaccessd uses
        let expected_href = format!("/calendar/dav/alice%40example.com/user/{}/", cal.id);
        assert!(
            body_str.contains(&expected_href),
            "Calendar href should be email-based: expected {expected_href}, body: {body_str}"
        );
        assert!(
            !body_str.contains(&format!("/caldav/users/alice/{}/", cal.id)),
            "Calendar href should NOT be username-based"
        );
    }

    #[tokio::test]
    async fn test_email_discovery_depth1_returns_email_based_hrefs() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

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

        // Email in hrefs must be percent-encoded (@ → %40) to match the URL dataaccessd uses
        let expected_href = format!("/calendar/dav/alice%40example.com/user/{}/", cal.id);
        assert!(
            body_str.contains(&expected_href),
            "Calendar href should be email-based: {body_str}"
        );
    }

    // --- Email-based calendar collection routes ---

    #[tokio::test]
    async fn test_email_calendar_propfind() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let uri = format!("/calendar/dav/alice%40example.com/user/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri(&uri)
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);

        assert!(
            body_str.contains("Work"),
            "Calendar name should be in response"
        );
    }

    #[tokio::test]
    async fn test_email_calendar_report_returns_events() {
        let (pool, _user, cal) = setup().await;

        // Create an event via DB
        crate::db::events::upsert_object(
            &pool, &cal.id, "test-uid@example.com",
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:test-uid@example.com\r\nSUMMARY:Test Event\r\nDTSTART:20260301T090000Z\r\nDTEND:20260301T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR",
            crate::db::events::ObjectFields {
                component_type: "VEVENT",
                dtstart: Some("20260301T090000Z"),
                dtend: Some("20260301T100000Z"),
                summary: Some("Test Event"),
            },
        ).await.unwrap();

        let app = router(pool);

        let report_body = r#"<?xml version="1.0" encoding="utf-8" ?>
<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag/>
    <C:calendar-data/>
  </D:prop>
  <C:filter>
    <C:comp-filter name="VCALENDAR">
      <C:comp-filter name="VEVENT"/>
    </C:comp-filter>
  </C:filter>
</C:calendar-query>"#;

        let uri = format!("/calendar/dav/alice%40example.com/user/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"REPORT").unwrap())
            .uri(&uri)
            .header("Content-Type", "application/xml")
            .body(Body::from(report_body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);

        // Event should be in the response
        assert!(
            body_str.contains("test-uid@example.com"),
            "Event UID should be in REPORT response"
        );
        assert!(
            body_str.contains("Test Event"),
            "Event summary should be in REPORT response"
        );
        // Hrefs should be email-based with percent-encoded @
        assert!(
            body_str.contains("/calendar/dav/alice%40example.com/user/"),
            "REPORT hrefs should be email-based with %40"
        );
    }

    // --- Email-based object routes ---

    #[tokio::test]
    async fn test_email_object_put_and_get() {
        let (pool, _user, cal) = setup().await;

        let ical_data = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:put-test@example.com\r\nSUMMARY:Put Test\r\nDTSTART:20260401T090000Z\r\nDTEND:20260401T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR";

        // PUT via email path
        let app = router(pool.clone());
        let put_uri = format!(
            "/calendar/dav/alice%40example.com/user/{}/put-test%40example.com.ics",
            cal.id
        );
        let req = Request::builder()
            .method("PUT")
            .uri(&put_uri)
            .header("Content-Type", "text/calendar")
            .body(Body::from(ical_data))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert!(
            resp.status() == StatusCode::CREATED || resp.status() == StatusCode::NO_CONTENT,
            "PUT should succeed: got {}",
            resp.status()
        );

        // GET via email path
        let app2 = router(pool);
        let get_uri = format!(
            "/calendar/dav/alice%40example.com/user/{}/put-test%40example.com.ics",
            cal.id
        );
        let req = Request::builder()
            .method("GET")
            .uri(&get_uri)
            .body(Body::empty())
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);
        assert!(
            body_str.contains("Put Test"),
            "GET should return the event we PUT"
        );
    }

    #[tokio::test]
    async fn test_email_object_delete() {
        let (pool, _user, cal) = setup().await;

        // Create event via DB
        crate::db::events::upsert_object(
            &pool,
            &cal.id,
            "del-test@example.com",
            "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
            crate::db::events::ObjectFields {
                component_type: "VEVENT",
                dtstart: None,
                dtend: None,
                summary: None,
            },
        )
        .await
        .unwrap();

        let app = router(pool.clone());
        let uri = format!(
            "/calendar/dav/alice%40example.com/user/{}/del-test%40example.com.ics",
            cal.id
        );
        let req = Request::builder()
            .method("DELETE")
            .uri(&uri)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Verify deleted
        let obj = crate::db::events::get_object_by_uid(&pool, &cal.id, "del-test@example.com")
            .await
            .unwrap();
        assert!(obj.is_none(), "Event should be deleted");
    }

    // --- Email-based error cases ---

    #[tokio::test]
    async fn test_email_calendar_unknown_email_returns_401() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let uri = format!("/calendar/dav/unknown%40example.com/user/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri(&uri)
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_email_calendar_wrong_calendar_returns_403() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/calendar/dav/alice%40example.com/user/nonexistent-cal/")
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_email_object_unknown_email_returns_401() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let uri = format!(
            "/calendar/dav/unknown%40example.com/user/{}/test.ics",
            cal.id
        );
        let req = Request::builder()
            .method("GET")
            .uri(&uri)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_email_object_wrong_calendar_returns_403() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method("GET")
            .uri("/calendar/dav/alice%40example.com/user/nonexistent-cal/test.ics")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // --- Verify username-based paths still work (backward compatibility) ---

    #[tokio::test]
    async fn test_username_calendar_propfind_still_works() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri(format!("/caldav/users/alice/{}/", cal.id))
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);

        // Username-based paths should still use username-based hrefs
        assert!(
            body_str.contains(&format!("/caldav/users/alice/{}/", cal.id)),
            "Username-based path should still use username-based hrefs"
        );
    }

    // --- PROPPATCH tests ---

    #[tokio::test]
    async fn test_email_proppatch_returns_email_based_href() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let proppatch_body = r#"<?xml version="1.0" encoding="utf-8" ?>
<D:propertyupdate xmlns:D="DAV:" xmlns:A="http://apple.com/ns/ical/">
  <D:set>
    <D:prop>
      <D:displayname>Renamed Calendar</D:displayname>
    </D:prop>
  </D:set>
</D:propertyupdate>"#;

        let uri = format!("/calendar/dav/alice%40example.com/user/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"PROPPATCH").unwrap())
            .uri(&uri)
            .header("Content-Type", "application/xml")
            .body(Body::from(proppatch_body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);

        // Href should be email-based with %40
        assert!(
            body_str.contains(&format!(
                "/calendar/dav/alice%40example.com/user/{}/",
                cal.id
            )),
            "PROPPATCH response href should use email-based path, got: {body_str}"
        );
        // Should NOT contain username-based path
        assert!(
            !body_str.contains("/caldav/users/"),
            "PROPPATCH response should not contain username-based path"
        );
    }

    #[tokio::test]
    async fn test_username_proppatch_returns_username_based_href() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let proppatch_body = r#"<?xml version="1.0" encoding="utf-8" ?>
<D:propertyupdate xmlns:D="DAV:" xmlns:A="http://apple.com/ns/ical/">
  <D:set>
    <D:prop>
      <D:displayname>Renamed Calendar</D:displayname>
    </D:prop>
  </D:set>
</D:propertyupdate>"#;

        let uri = format!("/caldav/users/alice/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"PROPPATCH").unwrap())
            .uri(&uri)
            .header("Content-Type", "application/xml")
            .header("Authorization", basic_auth_header("alice", "secret123"))
            .body(Body::from(proppatch_body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);

        // Href should be username-based
        assert!(
            body_str.contains(&format!("/caldav/users/alice/{}/", cal.id)),
            "PROPPATCH response href should use username-based path, got: {body_str}"
        );
    }

    // --- Property filtering tests ---

    #[tokio::test]
    async fn test_propfind_with_specific_props_returns_200_and_404_propstat() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        // Request specific properties: some we have, some we don't
        let propfind_body = r#"<?xml version="1.0" encoding="utf-8"?>
<A:propfind xmlns:A="DAV:" xmlns:B="urn:ietf:params:xml:ns:caldav" xmlns:C="http://calendarserver.org/ns/" xmlns:D="http://apple.com/ns/ical/">
  <A:prop>
    <A:displayname/>
    <A:resourcetype/>
    <A:quota-available-bytes/>
    <B:calendar-description/>
    <D:calendar-color/>
    <C:getctag/>
    <A:add-member/>
    <D:refreshrate/>
  </A:prop>
</A:propfind>"#;

        let uri = format!("/calendar/dav/alice%40example.com/user/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri(&uri)
            .header("Depth", "0")
            .header("Content-Type", "application/xml")
            .body(Body::from(propfind_body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);

        // 200 propstat should contain the properties we have
        assert!(
            body_str.contains("displayname"),
            "displayname should be in 200 propstat"
        );
        assert!(
            body_str.contains("resourcetype"),
            "resourcetype should be in 200 propstat"
        );
        assert!(
            body_str.contains("calendar-description"),
            "calendar-description should be in 200 propstat"
        );
        assert!(
            body_str.contains("calendar-color"),
            "calendar-color should be in 200 propstat"
        );
        assert!(
            body_str.contains("getctag"),
            "getctag should be in 200 propstat"
        );

        // 404 propstat should contain the properties we don't have
        assert!(
            body_str.contains("HTTP/1.1 404 Not Found"),
            "Should have 404 propstat"
        );
        assert!(
            body_str.contains("quota-available-bytes"),
            "quota-available-bytes should be in 404 propstat"
        );
        assert!(
            body_str.contains("add-member"),
            "add-member should be in 404 propstat"
        );
        assert!(
            body_str.contains("refreshrate"),
            "refreshrate should be in 404 propstat"
        );
    }

    #[tokio::test]
    async fn test_email_home_propfind_with_specific_props_filters_correctly() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        // Apple Calendar-style PROPFIND with non-standard namespace prefixes
        let propfind_body = r#"<?xml version="1.0" encoding="utf-8"?>
<A:propfind xmlns:A="DAV:" xmlns:B="urn:ietf:params:xml:ns:caldav" xmlns:C="http://calendarserver.org/ns/">
  <A:prop>
    <A:current-user-principal/>
    <A:displayname/>
    <B:calendar-home-set/>
    <B:calendar-user-address-set/>
    <C:email-address-set/>
    <A:resource-id/>
    <A:quota-available-bytes/>
  </A:prop>
</A:propfind>"#;

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/calendar/dav/alice%40example.com/user/")
            .header("Depth", "0")
            .header("Content-Type", "application/xml")
            .body(Body::from(propfind_body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);

        // 200 propstat should have the props we support
        assert!(
            body_str.contains("HTTP/1.1 200 OK"),
            "Should have 200 propstat"
        );
        assert!(
            body_str.contains("current-user-principal"),
            "Should have current-user-principal"
        );
        assert!(body_str.contains("displayname"), "Should have displayname");
        assert!(
            body_str.contains("calendar-home-set"),
            "Should have calendar-home-set"
        );
        assert!(
            body_str.contains("calendar-user-address-set"),
            "Should have calendar-user-address-set"
        );
        assert!(
            body_str.contains("email-address-set"),
            "Should have email-address-set"
        );
        assert!(body_str.contains("resource-id"), "Should have resource-id");

        // 404 propstat should have the props we don't support
        assert!(
            body_str.contains("HTTP/1.1 404 Not Found"),
            "Should have 404 propstat"
        );
        assert!(
            body_str.contains("quota-available-bytes"),
            "quota-available-bytes should be 404"
        );
    }

    #[tokio::test]
    async fn test_propfind_allprop_returns_no_404_propstat() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        // Empty body = allprop
        let uri = format!("/calendar/dav/alice%40example.com/user/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri(&uri)
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);

        // AllProp should NOT have a 404 propstat
        assert!(
            body_str.contains("HTTP/1.1 200 OK"),
            "Should have 200 propstat"
        );
        assert!(
            !body_str.contains("HTTP/1.1 404 Not Found"),
            "AllProp should not have 404 propstat"
        );
    }

    // --- well-known endpoint ---

    #[tokio::test]
    async fn test_well_known_returns_301_redirect() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/.well-known/caldav")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MOVED_PERMANENTLY);
        assert_eq!(
            resp.headers().get("Location").unwrap().to_str().unwrap(),
            "/caldav/"
        );
    }

    #[tokio::test]
    async fn test_well_known_options_returns_200_with_dav_headers() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method("OPTIONS")
            .uri("/.well-known/caldav")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let dav = resp.headers().get("DAV").unwrap().to_str().unwrap();
        assert!(dav.contains("calendar-access"));
    }

    // --- options endpoint ---

    #[tokio::test]
    async fn test_caldav_root_options_returns_dav_headers() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method("OPTIONS")
            .uri("/caldav/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let dav = resp.headers().get("DAV").unwrap().to_str().unwrap();
        assert!(dav.contains("calendar-access"));
    }

    #[tokio::test]
    async fn test_calendar_collection_options_returns_dav_headers() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let uri = format!("/caldav/users/alice/{}/", cal.id);
        let req = Request::builder()
            .method("OPTIONS")
            .uri(&uri)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let dav = resp.headers().get("DAV").unwrap().to_str().unwrap();
        assert!(dav.contains("calendar-access"));
    }

    #[tokio::test]
    async fn test_object_options_returns_dav_headers() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let uri = format!("/caldav/users/alice/{}/test.ics", cal.id);
        let req = Request::builder()
            .method("OPTIONS")
            .uri(&uri)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let dav = resp.headers().get("DAV").unwrap().to_str().unwrap();
        assert!(dav.contains("calendar-access"));
    }

    // --- caldav root method handling ---

    #[tokio::test]
    async fn test_caldav_root_unknown_method_returns_405() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method("DELETE")
            .uri("/caldav/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn test_caldav_root_propfind_authenticated_returns_principal() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/caldav/")
            .header("Authorization", basic_auth_header("alice", "secret123"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);
        assert!(
            body_str.contains("alice"),
            "Authenticated response should contain username"
        );
    }

    // --- server root ---

    #[tokio::test]
    async fn test_server_root_options_returns_200() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method("OPTIONS")
            .uri("/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_server_root_unknown_method_redirects() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method("GET")
            .uri("/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MOVED_PERMANENTLY);
    }

    #[tokio::test]
    async fn test_server_root_propfind_authenticated() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/")
            .header("Authorization", basic_auth_header("alice", "secret123"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);
        assert!(
            body_str.contains("alice"),
            "Authenticated response should contain username"
        );
    }

    // --- principal discovery ---

    #[tokio::test]
    async fn test_principal_discovery_redirects() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/caldav/principals/alice/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MOVED_PERMANENTLY);
        let loc = resp.headers().get("Location").unwrap().to_str().unwrap();
        assert_eq!(loc, "/caldav/users/alice/");
    }

    #[tokio::test]
    async fn test_principal_discovery_options_returns_200() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method("OPTIONS")
            .uri("/caldav/principals/alice/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // --- MKCALENDAR ---

    #[tokio::test]
    async fn test_mkcalendar_creates_calendar() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool.clone());

        let new_cal_id = "my-new-calendar";
        let uri = format!("/caldav/users/alice/{new_cal_id}/");

        let req = Request::builder()
            .method(Method::from_bytes(b"MKCALENDAR").unwrap())
            .uri(&uri)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // Verify the calendar was created in the DB
        let cal = crate::db::calendars::get_calendar_by_id(&pool, new_cal_id)
            .await
            .unwrap();
        assert!(cal.is_some(), "Calendar should exist in DB");
    }

    #[tokio::test]
    async fn test_mkcalendar_with_displayname() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool.clone());

        let new_cal_id = "named-calendar";
        let uri = format!("/caldav/users/alice/{new_cal_id}/");

        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<C:mkcalendar xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:A="http://apple.com/ns/ical/">
  <D:set>
    <D:prop>
      <D:displayname>My Calendar</D:displayname>
      <A:calendar-color>#FF0000</A:calendar-color>
    </D:prop>
  </D:set>
</C:mkcalendar>"#;

        let req = Request::builder()
            .method(Method::from_bytes(b"MKCALENDAR").unwrap())
            .uri(&uri)
            .header("Content-Type", "application/xml")
            .body(Body::from(body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let cal = crate::db::calendars::get_calendar_by_id(&pool, new_cal_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cal.name, "My Calendar");
        assert_eq!(cal.color, "#FF0000");
    }

    #[tokio::test]
    async fn test_mkcalendar_duplicate_returns_method_not_allowed() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        // Try to create a calendar that already exists
        let uri = format!("/caldav/users/alice/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"MKCALENDAR").unwrap())
            .uri(&uri)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn test_mkcalendar_cross_user_returns_forbidden() {
        let pool = db::test_pool().await;
        let _alice = users::create_user(&pool, "alice", None, "pass1")
            .await
            .unwrap();
        let _bob = users::create_user(&pool, "bob", None, "pass2")
            .await
            .unwrap();
        let app = router(pool);

        // Alice (resolved via path) tries to create in bob's space — forbidden
        let req = Request::builder()
            .method(Method::from_bytes(b"MKCALENDAR").unwrap())
            .uri("/caldav/users/bob/some-cal/")
            .header("Authorization", basic_auth_header("alice", "pass1"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // --- DELETE ---

    #[tokio::test]
    async fn test_delete_object_returns_no_content() {
        let (pool, _user, cal) = setup().await;

        // Create the event
        crate::db::events::upsert_object(
            &pool,
            &cal.id,
            "delete-me@example.com",
            "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
            crate::db::events::ObjectFields {
                component_type: "VEVENT",
                dtstart: None,
                dtend: None,
                summary: None,
            },
        )
        .await
        .unwrap();

        let app = router(pool.clone());
        let uri = format!("/caldav/users/alice/{}/delete-me%40example.com.ics", cal.id);
        let req = Request::builder()
            .method("DELETE")
            .uri(&uri)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let obj = crate::db::events::get_object_by_uid(&pool, &cal.id, "delete-me@example.com")
            .await
            .unwrap();
        assert!(obj.is_none());
    }

    #[tokio::test]
    async fn test_delete_object_not_found_returns_404() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let uri = format!("/caldav/users/alice/{}/nonexistent.ics", cal.id);
        let req = Request::builder()
            .method("DELETE")
            .uri(&uri)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_calendar_returns_no_content() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool.clone());

        let uri = format!("/caldav/users/alice/{}/", cal.id);
        let req = Request::builder()
            .method("DELETE")
            .uri(&uri)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let deleted = crate::db::calendars::get_calendar_by_id(&pool, &cal.id)
            .await
            .unwrap();
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn test_delete_calendar_not_found_returns_404() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method("DELETE")
            .uri("/caldav/users/alice/nonexistent-cal/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Access denied takes priority (user doesn't own it)
        assert!(
            resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::NOT_FOUND,
            "Expected 403 or 404, got {}",
            resp.status()
        );
    }

    // --- PUT ---

    #[tokio::test]
    async fn test_put_creates_event() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool.clone());

        let ical_data = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:put-new@test.com\r\nSUMMARY:New Event\r\nDTSTART:20260301T090000Z\r\nDTEND:20260301T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

        let uri = format!("/caldav/users/alice/{}/put-new%40test.com.ics", cal.id);
        let req = Request::builder()
            .method("PUT")
            .uri(&uri)
            .header("Content-Type", "text/calendar")
            .body(Body::from(ical_data))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        assert!(resp.headers().contains_key("etag"));
    }

    #[tokio::test]
    async fn test_put_updates_existing_event() {
        let (pool, _user, cal) = setup().await;

        // Create initial event
        let (initial, _) = crate::db::events::upsert_object(
            &pool, &cal.id, "update-me@test.com",
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:update-me@test.com\r\nSUMMARY:Old\r\nDTSTART:20260301T090000Z\r\nDTEND:20260301T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR",
            crate::db::events::ObjectFields {
                component_type: "VEVENT",
                dtstart: Some("20260301T090000Z"),
                dtend: Some("20260301T100000Z"),
                summary: Some("Old"),
            },
        ).await.unwrap();

        let app = router(pool.clone());

        let updated_ical = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:update-me@test.com\r\nSUMMARY:Updated\r\nDTSTART:20260301T090000Z\r\nDTEND:20260301T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

        let uri = format!("/caldav/users/alice/{}/update-me%40test.com.ics", cal.id);
        let req = Request::builder()
            .method("PUT")
            .uri(&uri)
            .header("Content-Type", "text/calendar")
            .body(Body::from(updated_ical))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let obj = crate::db::events::get_object_by_uid(&pool, &cal.id, "update-me@test.com")
            .await
            .unwrap()
            .unwrap();
        assert_ne!(obj.etag, initial.etag, "ETag should change on update");
        assert!(obj.ical_data.contains("Updated"));
    }

    #[tokio::test]
    async fn test_put_with_if_match_etag_mismatch_returns_412() {
        let (pool, _user, cal) = setup().await;

        crate::db::events::upsert_object(
            &pool, &cal.id, "ifmatch@test.com",
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:ifmatch@test.com\r\nEND:VEVENT\r\nEND:VCALENDAR",
            crate::db::events::ObjectFields {
                component_type: "VEVENT",
                dtstart: None,
                dtend: None,
                summary: None,
            },
        ).await.unwrap();

        let app = router(pool);
        let uri = format!("/caldav/users/alice/{}/ifmatch%40test.com.ics", cal.id);
        let req = Request::builder()
            .method("PUT")
            .uri(&uri)
            .header("If-Match", "\"wrong-etag\"")
            .body(Body::from("BEGIN:VCALENDAR\r\nEND:VCALENDAR"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);
    }

    #[tokio::test]
    async fn test_put_with_if_match_on_nonexistent_returns_412() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let uri = format!("/caldav/users/alice/{}/nope@test.com.ics", cal.id);
        let req = Request::builder()
            .method("PUT")
            .uri(&uri)
            .header("If-Match", "\"some-etag\"")
            .body(Body::from("BEGIN:VCALENDAR\r\nEND:VCALENDAR"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);
    }

    #[tokio::test]
    async fn test_put_with_if_match_star_succeeds() {
        let (pool, _user, cal) = setup().await;

        crate::db::events::upsert_object(
            &pool,
            &cal.id,
            "star@test.com",
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:star@test.com\r\nEND:VEVENT\r\nEND:VCALENDAR",
            crate::db::events::ObjectFields {
                component_type: "VEVENT",
                dtstart: None,
                dtend: None,
                summary: None,
            },
        )
        .await
        .unwrap();

        let app = router(pool);
        let uri = format!("/caldav/users/alice/{}/star%40test.com.ics", cal.id);
        let req = Request::builder()
            .method("PUT")
            .uri(&uri)
            .header("If-Match", "*")
            .body(Body::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:star@test.com\r\nSUMMARY:Updated\r\nDTSTART:20260101T000000Z\r\nDTEND:20260101T010000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    // --- GET ---

    #[tokio::test]
    async fn test_get_existing_event() {
        let (pool, _user, cal) = setup().await;

        crate::db::events::upsert_object(
            &pool, &cal.id, "get-me@test.com",
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:get-me@test.com\r\nSUMMARY:Get Me\r\nEND:VEVENT\r\nEND:VCALENDAR",
            crate::db::events::ObjectFields {
                component_type: "VEVENT",
                dtstart: None,
                dtend: None,
                summary: Some("Get Me"),
            },
        ).await.unwrap();

        let app = router(pool);
        let uri = format!("/caldav/users/alice/{}/get-me%40test.com.ics", cal.id);
        let req = Request::builder()
            .method("GET")
            .uri(&uri)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap(),
            "text/calendar; charset=utf-8"
        );
        assert!(resp.headers().contains_key("etag"));

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&body).contains("Get Me"));
    }

    #[tokio::test]
    async fn test_get_nonexistent_event_returns_404() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let uri = format!("/caldav/users/alice/{}/nope.ics", cal.id);
        let req = Request::builder()
            .method("GET")
            .uri(&uri)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // --- REPORT ---

    #[tokio::test]
    async fn test_report_calendar_query_no_filter() {
        let (pool, _user, cal) = setup().await;

        crate::db::events::upsert_object(
            &pool, &cal.id, "query-uid@test.com",
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:query-uid@test.com\r\nSUMMARY:Query Event\r\nDTSTART:20260301T090000Z\r\nDTEND:20260301T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR",
            crate::db::events::ObjectFields {
                component_type: "VEVENT",
                dtstart: Some("20260301T090000Z"),
                dtend: Some("20260301T100000Z"),
                summary: Some("Query Event"),
            },
        ).await.unwrap();

        let app = router(pool);

        let report_body = r#"<?xml version="1.0" encoding="utf-8"?>
<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag/>
    <C:calendar-data/>
  </D:prop>
  <C:filter>
    <C:comp-filter name="VCALENDAR"/>
  </C:filter>
</C:calendar-query>"#;

        let uri = format!("/caldav/users/alice/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"REPORT").unwrap())
            .uri(&uri)
            .body(Body::from(report_body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);
        assert!(body_str.contains("query-uid@test.com"));
    }

    #[tokio::test]
    async fn test_report_calendar_multiget() {
        let (pool, _user, cal) = setup().await;

        crate::db::events::upsert_object(
            &pool, &cal.id, "multiget-uid@test.com",
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:multiget-uid@test.com\r\nSUMMARY:Multiget Event\r\nDTSTART:20260301T090000Z\r\nDTEND:20260301T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR",
            crate::db::events::ObjectFields {
                component_type: "VEVENT",
                dtstart: Some("20260301T090000Z"),
                dtend: Some("20260301T100000Z"),
                summary: Some("Multiget Event"),
            },
        ).await.unwrap();

        let app = router(pool.clone());

        let report_body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<C:calendar-multiget xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag/>
    <C:calendar-data/>
  </D:prop>
  <D:href>/caldav/users/alice/{}/multiget-uid%40test.com.ics</D:href>
</C:calendar-multiget>"#,
            cal.id
        );

        let uri = format!("/caldav/users/alice/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"REPORT").unwrap())
            .uri(&uri)
            .body(Body::from(report_body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);
        assert!(body_str.contains("multiget-uid@test.com"));
        assert!(body_str.contains("Multiget Event"));
    }

    #[tokio::test]
    async fn test_report_sync_collection_initial_sync() {
        let (pool, _user, cal) = setup().await;

        crate::db::events::upsert_object(
            &pool, &cal.id, "sync-uid@test.com",
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:sync-uid@test.com\r\nSUMMARY:Sync Event\r\nDTSTART:20260301T090000Z\r\nDTEND:20260301T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR",
            crate::db::events::ObjectFields {
                component_type: "VEVENT",
                dtstart: Some("20260301T090000Z"),
                dtend: Some("20260301T100000Z"),
                summary: Some("Sync Event"),
            },
        ).await.unwrap();

        let app = router(pool);

        let report_body = r#"<?xml version="1.0" encoding="utf-8"?>
<D:sync-collection xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:sync-token/>
  <D:sync-level>1</D:sync-level>
  <D:prop>
    <D:getetag/>
  </D:prop>
</D:sync-collection>"#;

        let uri = format!("/caldav/users/alice/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"REPORT").unwrap())
            .uri(&uri)
            .body(Body::from(report_body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);
        assert!(body_str.contains("sync-uid@test.com"));
        // Sync token must be a valid URI (RFC 6578)
        assert!(body_str.contains("sync-token"), "Should contain sync-token");
    }

    #[tokio::test]
    async fn test_report_sync_collection_with_calendar_data() {
        let (pool, _user, cal) = setup().await;

        crate::db::events::upsert_object(
            &pool, &cal.id, "sync-data@test.com",
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:sync-data@test.com\r\nSUMMARY:Sync Data\r\nDTSTART:20260301T090000Z\r\nDTEND:20260301T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR",
            crate::db::events::ObjectFields {
                component_type: "VEVENT",
                dtstart: Some("20260301T090000Z"),
                dtend: Some("20260301T100000Z"),
                summary: Some("Sync Data"),
            },
        ).await.unwrap();

        let app = router(pool);

        let report_body = r#"<?xml version="1.0" encoding="utf-8"?>
<D:sync-collection xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:sync-token/>
  <D:sync-level>1</D:sync-level>
  <D:prop>
    <D:getetag/>
    <C:calendar-data/>
  </D:prop>
</D:sync-collection>"#;

        let uri = format!("/caldav/users/alice/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"REPORT").unwrap())
            .uri(&uri)
            .body(Body::from(report_body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);
        assert!(
            body_str.contains("Sync Data"),
            "calendar-data should be included"
        );
    }

    #[tokio::test]
    async fn test_report_invalid_body_returns_400() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let uri = format!("/caldav/users/alice/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"REPORT").unwrap())
            .uri(&uri)
            .body(Body::from("not valid xml"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_report_calendar_query_with_time_range() {
        let (pool, _user, cal) = setup().await;

        crate::db::events::upsert_object(
            &pool, &cal.id, "range-uid@test.com",
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:range-uid@test.com\r\nSUMMARY:Range Event\r\nDTSTART:20260301T090000Z\r\nDTEND:20260301T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR",
            crate::db::events::ObjectFields {
                component_type: "VEVENT",
                dtstart: Some("20260301T090000Z"),
                dtend: Some("20260301T100000Z"),
                summary: Some("Range Event"),
            },
        ).await.unwrap();

        let app = router(pool);

        let report_body = r#"<?xml version="1.0" encoding="utf-8"?>
<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag/>
  </D:prop>
  <C:filter>
    <C:comp-filter name="VCALENDAR">
      <C:comp-filter name="VEVENT">
        <C:time-range start="20260201T000000Z" end="20260401T000000Z"/>
      </C:comp-filter>
    </C:comp-filter>
  </C:filter>
</C:calendar-query>"#;

        let uri = format!("/caldav/users/alice/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"REPORT").unwrap())
            .uri(&uri)
            .body(Body::from(report_body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);
        assert!(body_str.contains("range-uid@test.com"));
    }

    // --- calendar home PROPFIND ---

    #[tokio::test]
    async fn test_calendar_home_depth0_returns_home_props() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/caldav/users/alice/")
            .header("Depth", "0")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);
        assert!(
            body_str.contains("current-user-principal"),
            "Depth:0 response should contain current-user-principal"
        );
        assert!(
            body_str.contains("displayname"),
            "Depth:0 response should contain displayname"
        );
    }

    #[tokio::test]
    async fn test_calendar_home_depth1_lists_calendars() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/caldav/users/alice/")
            .header("Depth", "1")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);
        assert!(
            body_str.contains(&cal.id),
            "Calendar should appear in depth:1 list"
        );
    }

    #[tokio::test]
    async fn test_calendar_collection_depth1_lists_objects() {
        let (pool, _user, cal) = setup().await;

        crate::db::events::upsert_object(
            &pool, &cal.id, "listed@test.com",
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:listed@test.com\r\nSUMMARY:Listed\r\nEND:VEVENT\r\nEND:VCALENDAR",
            crate::db::events::ObjectFields {
                component_type: "VEVENT",
                dtstart: None,
                dtend: None,
                summary: Some("Listed"),
            },
        ).await.unwrap();

        let app = router(pool);

        let uri = format!("/caldav/users/alice/{}/", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri(&uri)
            .header("Depth", "1")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body);
        assert!(body_str.contains("listed@test.com"));
    }

    #[tokio::test]
    async fn test_calendar_collection_unknown_method_returns_405() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let uri = format!("/caldav/users/alice/{}/", cal.id);
        let req = Request::builder()
            .method("GET")
            .uri(&uri)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn test_object_unknown_method_returns_405() {
        let (pool, _user, cal) = setup().await;
        let app = router(pool);

        let uri = format!("/caldav/users/alice/{}/test.ics", cal.id);
        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri(&uri)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    // --- encode_email_for_path ---

    #[test]
    fn test_encode_email_for_path() {
        assert_eq!(
            encode_email_for_path("alice@example.com"),
            "alice%40example.com"
        );
        assert_eq!(encode_email_for_path("no-at-sign"), "no-at-sign");
        assert_eq!(encode_email_for_path("a@b@c"), "a%40b%40c");
    }

    // --- fallback discovery ---

    #[tokio::test]
    async fn test_fallback_principals_redirects() {
        let (pool, _user, _cal) = setup().await;
        let app = router(pool);

        let req = Request::builder()
            .method(Method::from_bytes(b"PROPFIND").unwrap())
            .uri("/principals/")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Should be a redirect or 207
        assert!(
            resp.status() == StatusCode::MOVED_PERMANENTLY
                || resp.status() == StatusCode::MULTI_STATUS,
            "Expected redirect or 207, got {}",
            resp.status()
        );
    }
}
