# CalDAV Server — Handler Flow Reference

This document captures the decision points, process steps, and terminals for every handler in the CalDAV server. Use this as input when generating flowchart SVGs.

## Diagram Index

Each SVG in `docs/images/` is generated from the sections listed below.

| Diagram | File | Source Sections |
|---------|------|-----------------|
| Well-Known Discovery | [`flow-wellknown-discovery.svg`](images/flow-wellknown-discovery.svg) | §1, §2, §3, §4, §5, §6, §7 |
| PROPFIND | [`flow-propfind.svg`](images/flow-propfind.svg) | §8, §17 |
| GET Event | [`flow-get-event.svg`](images/flow-get-event.svg) | §11 |
| PUT Event | [`flow-put-event.svg`](images/flow-put-event.svg) | §12 |
| DELETE | [`flow-delete.svg`](images/flow-delete.svg) | §13, §14 |
| MKCALENDAR | [`flow-mkcalendar.svg`](images/flow-mkcalendar.svg) | §15 |
| PROPPATCH | [`flow-proppatch.svg`](images/flow-proppatch.svg) | §16 |
| REPORT | [`flow-report.svg`](images/flow-report.svg) | §18 |
| MCP Request | [`flow-mcp-request.svg`](images/flow-mcp-request.svg) | §21 |
| Authentication | [`flow-authentication.svg`](images/flow-authentication.svg) | §22 |
| Database Schema | [`schema.svg`](images/schema.svg) | n/a (see `migrations/001_initial.sql`) |

**Sections without diagrams:** §9 (Calendar Collection routing), §10 (Calendar Object routing), §19 (Email Calendar Collection), §20 (Email Object), §23 (Apple Calendar Discovery & Sync). Sections 9–10 and 19–20 are routing dispatch handlers that delegate to the diagrammed flows above. Section 23 describes a protocol sequence rather than a code flowchart.

---

## 1. Well-Known Discovery (`/.well-known/caldav`)

**Entry Point:** Any HTTP method on `/.well-known/caldav`

**Flow:**
1. Log request details (method, URI, auth, user-agent)
2. **Decision: Is method OPTIONS?**
   - **YES → Terminal: 200 OK** with DAV and Allow headers
   - **NO → Continue**
3. **Terminal: 301 MOVED_PERMANENTLY** redirect to `/caldav/` with DAV headers

---

## 2. OPTIONS (All CalDAV paths)

**Entry Point:** OPTIONS method on any CalDAV path

**Flow:**
1. **Terminal: 200 OK** with:
   - DAV header: "1, 2, 3, calendar-access, calendar-schedule"
   - Allow header: "OPTIONS, GET, HEAD, PUT, DELETE, PROPFIND, PROPPATCH, REPORT, MKCALENDAR"

---

## 3. Server Root (`GET /`)

**Entry Point:** Any method on `/`

**Flow:**
1. Extract auth header
2. Log request (method, URI, auth present, user-agent)
3. **Decision: Is method OPTIONS?**
   - **YES → Terminal: 200 OK** OPTIONS response
   - **NO → Continue**
4. **Decision: Is method PROPFIND?**
   - **YES → Continue to PROPFIND processing**
   - **NO → Terminal: 301 MOVED_PERMANENTLY** to `/caldav/`
5. **PROPFIND processing:**
   - Build multistatus response
   - **Decision: Auth header present AND valid?**
     - **YES → Add authenticated root props (with username)** → Terminal: 207 MULTI_STATUS
     - **NO → Add unauthenticated root props** → Terminal: 207 MULTI_STATUS

---

## 4. CalDAV Root (`/caldav/`)

**Entry Point:** Any method on `/caldav/` or `/caldav`

**Flow:**
1. Extract auth header
2. Log request (method, URI, auth present, user-agent)
3. **Decision: Is method OPTIONS?**
   - **YES → Terminal: 200 OK** OPTIONS response
   - **NO → Continue**
4. **Decision: Is method PROPFIND?**
   - **YES → Continue to PROPFIND processing**
   - **NO → Terminal: 405 METHOD_NOT_ALLOWED**
5. **PROPFIND processing:**
   - Build multistatus response
   - **Decision: Auth header present AND valid?**
     - **YES → Add authenticated root props** → Terminal: 207 MULTI_STATUS
     - **NO → Add unauthenticated root props** → Terminal: 207 MULTI_STATUS

---

## 5. Principal Discovery (`/caldav/principals/{username}/`)

**Entry Point:** Any method on `/caldav/principals/{username}/`

**Flow:**
1. Log request (method, URI, username)
2. **Decision: Is method OPTIONS?**
   - **YES → Terminal: 200 OK** OPTIONS response
   - **NO → Terminal: 301 MOVED_PERMANENTLY** to `/caldav/users/{username}/`

---

## 6. Fallback Discovery (`/principals/`, `/principals/{username}/`)

**Entry Point:** Any method on `/principals/` or `/principals/{username}/`

**Flow:**
1. Extract auth header
2. Log request (method, URI, auth present)
3. **Decision: Is method OPTIONS?**
   - **YES → Terminal: 200 OK** OPTIONS response
   - **NO → Continue**
4. **Decision: Is method PROPFIND?**
   - **YES → Continue to PROPFIND processing**
   - **NO → Terminal: 405 METHOD_NOT_ALLOWED**
5. **PROPFIND processing:**
   - Build multistatus response
   - **Decision: inline_auth() succeeds?**
     - **YES → Add authenticated props** → Terminal: 207 MULTI_STATUS
     - **NO → Add unauthenticated props** → Terminal: 207 MULTI_STATUS

---

## 7. Email Discovery (`/calendar/dav/{email}/user/`)

**Entry Point:** Any method on `/calendar/dav/{email}/user/`

**Flow:**
1. Extract auth header, depth, and request body
2. Encode email for path
3. Log request (method, email, depth, auth present, body)
4. **Decision: Is method OPTIONS?**
   - **YES → Terminal: 200 OK** OPTIONS response
   - **NO → Continue**
5. **Decision: Is method PROPFIND?**
   - **YES → Continue**
   - **NO → Terminal: 405 METHOD_NOT_ALLOWED**
6. Parse PROPFIND request
7. **Decision: Auth header present?**
   - **YES → Continue to auth validation**
   - **NO → Continue to email-based user lookup**
8. **Auth validation path:**
   - **Decision: try_basic_auth() succeeds?**
     - **YES → user found** → Continue to handle_email_home
     - **NO → Terminal: 401 UNAUTHORIZED**
9. **Email lookup path (no auth):**
   - **Decision: User found by email?**
     - **YES → Continue to handle_email_home**
     - **NO → Terminal: 207 MULTI_STATUS** (generic unauthenticated response, anti-enumeration)
10. **handle_email_home:** Build response with email context
    - Add email home props (with request_path)
    - **Decision: Depth >= 1?**
      - **YES → Query calendars, add to response**
      - **NO → Skip calendar list**
    - **Terminal: 207 MULTI_STATUS** with email-based hrefs

---

## 8. Calendar Home PROPFIND (`/caldav/users/{username}/`)

**Entry Point:** PROPFIND method on `/caldav/users/{username}/`

**Flow (outer handler `handle_calendar_home_route`):**
1. Extract auth header
2. **Decision: Is method OPTIONS?**
   - **YES → Terminal: 200 OK** OPTIONS response
   - **NO → Continue**
3. **Decision: auth_or_path_user() succeeds?**
   - **NO → Terminal: 401 UNAUTHORIZED**
   - **YES → Continue**
4. Insert user into request extensions
5. **Decision: Is method PROPFIND?**
   - **YES → Call handle_calendar_home() (propfind.rs)**
   - **NO → Terminal: 405 METHOD_NOT_ALLOWED**

**Flow (inner handler `handle_calendar_home` in propfind.rs):**
1. Extract authenticated user from request extensions
2. Extract Depth header (0 or 1, default 0)
3. Parse request body for PROPFIND request
4. Build multistatus response
5. Filter requested properties against calendar_home_props
6. Add home resource to response (HTTP 200 + 404 propstat)
7. **Decision: Depth >= 1?**
   - **NO → Terminal: 207 MULTI_STATUS** (just home)
   - **YES → Continue**
8. Query calendars::list_calendars_for_user()
9. For each calendar: filter properties, add to response
10. **Terminal: 207 MULTI_STATUS** (home + calendars)

---

## 9. Calendar Collection (`/caldav/users/{username}/{calendar_id}/`)

**Entry Point:** Any method on `/caldav/users/{username}/{calendar_id}/`

**Flow (outer handler `handle_calendar_collection`):**
1. Extract auth header
2. **Decision: Is method OPTIONS?**
   - **YES → Terminal: 200 OK** OPTIONS response
   - **NO → Continue**
3. **Decision: auth_or_path_user() succeeds?**
   - **NO → Terminal: 401 UNAUTHORIZED**
   - **YES → Continue**
4. **Decision: Method is MKCALENDAR?**
   - **YES → Skip access check** → Continue to method dispatch
   - **NO → Verify calendar access** → Continue
5. **Verify calendar access:**
   - **Decision: User has access to calendar?**
     - **NO → Terminal: 403 FORBIDDEN**
     - **YES → Continue**
6. Insert user into request extensions
7. **Method dispatch:**
   - **PROPFIND → Call handle_calendar() → Terminal: 207 MULTI_STATUS**
   - **REPORT → Call handle_report() → Terminal: 207 MULTI_STATUS**
   - **MKCALENDAR → Call handle_mkcalendar() → Terminal: 201 CREATED**
   - **PROPPATCH → Call handle_proppatch() → Terminal: 207 MULTI_STATUS**
   - **DELETE → Call handle_delete_calendar() → Terminal: 204 NO_CONTENT**
   - **Other → Terminal: 405 METHOD_NOT_ALLOWED**

---

## 10. Calendar Object (`/caldav/users/{username}/{calendar_id}/{filename}`)

**Entry Point:** Any method on `/caldav/users/{username}/{calendar_id}/{filename}`

**Flow (outer handler `handle_object`):**
1. Extract auth header
2. **Decision: Is method OPTIONS?**
   - **YES → Terminal: 200 OK** OPTIONS response
   - **NO → Continue**
3. **Decision: auth_or_path_user() succeeds?**
   - **NO → Terminal: 401 UNAUTHORIZED**
   - **YES → Continue**
4. **Decision: User has access to calendar?**
   - **NO → Terminal: 403 FORBIDDEN**
   - **YES → Continue**
5. Insert user into request extensions
6. **Method dispatch:**
   - **GET → Call handle_get() → Terminal: 200 OK**
   - **PUT → Call handle_put() → Terminal: 201 CREATED or 204 NO_CONTENT**
   - **DELETE → Call handle_delete_object() → Terminal: 204 NO_CONTENT**
   - **Other → Terminal: 405 METHOD_NOT_ALLOWED**

---

## 11. GET Event (`GET /{username}/{calendar_id}/{uid}.ics`)

**Entry Point:** GET method on calendar object (dispatched from handle_object)

**Flow:**
1. Extract UID from filename (strip .ics extension)
2. **Decision: events::get_object_by_uid() succeeds?**
   - **DB Error → Terminal: 500 INTERNAL_SERVER_ERROR** "Internal error"
   - **OK(None) → Terminal: 404 NOT_FOUND** "Object not found"
   - **OK(Some(obj)) → Continue**
3. **Terminal: 200 OK** with:
   - Content-Type: text/calendar; charset=utf-8
   - ETag header: object.etag
   - Body: object.ical_data

---

## 12. PUT Event (`PUT /{username}/{calendar_id}/{uid}.ics`)

**Entry Point:** PUT method on calendar object (dispatched from handle_object)

**Flow:**
1. Extract UID from filename
2. Extract If-Match header (conditional update)
3. **Decision: Body read succeeds?**
   - **NO → Terminal: 400 BAD_REQUEST** "Request body too large"
   - **YES → Continue**
4. **Decision: Body is valid UTF-8?**
   - **NO → Terminal: 400 BAD_REQUEST** "Invalid UTF-8"
   - **YES → Continue**
5. Parse iCalendar fields from body
6. Determine UID (from body or URL)
7. **Decision: If-Match header present?**
   - **NO → Skip validation** → Continue to upsert
   - **YES → Continue to validation**
8. **If-Match validation:**
   - **Decision: If-Match is "*"?**
     - **YES → Skip ETag check** → Continue to upsert
     - **NO → Check ETag match**
   - **Decision: events::get_object_by_uid() fails?**
     - **YES → Terminal: 500 INTERNAL_SERVER_ERROR** "Internal error"
     - **NO → Continue**
   - **Decision: Object doesn't exist (Ok(None))?**
     - **YES → Terminal: 412 PRECONDITION_FAILED** "Object does not exist"
     - **NO → Continue**
   - **Decision: ETag matches expected?**
     - **NO → Terminal: 412 PRECONDITION_FAILED** "ETag mismatch"
     - **YES → Continue to upsert**
9. **Upsert process:**
   - **Decision: events::upsert_object() succeeds?**
     - **YES → (obj, is_new) returned** → Continue
     - **NO → Terminal: 500 INTERNAL_SERVER_ERROR** "Failed to save event"
10. **Decision: is_new?**
    - **YES → Status = 201 CREATED**
    - **NO → Status = 204 NO_CONTENT**
11. **Terminal: 201/204** with ETag header

---

## 13. DELETE Object (`DELETE /{username}/{calendar_id}/{uid}.ics`)

**Entry Point:** DELETE method on calendar object (dispatched from handle_object)

**Flow:**
1. Extract UID from filename
2. **Decision: events::delete_object() succeeds?**
   - **YES → Terminal: 204 NO_CONTENT**
   - **AppError::NotFound → Terminal: 404 NOT_FOUND** "Object not found"
   - **Other error → Terminal: 500 INTERNAL_SERVER_ERROR** "Internal error"

---

## 14. DELETE Calendar (`DELETE /{username}/{calendar_id}/`)

**Entry Point:** DELETE method on calendar collection (dispatched from handle_calendar_collection)

**Flow:**
1. **Decision: calendars::delete_calendar() succeeds?**
   - **YES → Terminal: 204 NO_CONTENT**
   - **AppError::NotFound → Terminal: 404 NOT_FOUND** "Calendar not found"
   - **Other error → Terminal: 500 INTERNAL_SERVER_ERROR** "Internal error"

---

## 15. MKCALENDAR (`MKCALENDAR /{username}/{calendar_id}/`)

**Entry Point:** MKCALENDAR method on calendar path (dispatched from handle_calendar_collection)

**Flow:**
1. Extract authenticated user from request extensions
2. **Decision: Authenticated user matches path username?**
   - **NO → Terminal: 403 FORBIDDEN** "Cannot create calendars for another user"
   - **YES → Continue**
3. **Decision: Calendar already exists?**
   - **YES → Terminal: 405 METHOD_NOT_ALLOWED** "Calendar already exists"
   - **NO → Continue**
4. Extract request body
5. Parse optional calendar properties from body:
   - Extract displayname (default: calendar_id)
   - Extract calendar-color (default: #0E61B9)
6. **Decision: calendars::create_calendar_with_id() succeeds?**
   - **YES → Terminal: 201 CREATED** "Calendar created"
   - **NO → Terminal: 500 INTERNAL_SERVER_ERROR** "Failed to create calendar"

---

## 16. PROPPATCH (`PROPPATCH /{username}/{calendar_id}/`)

**Entry Point:** PROPPATCH method on calendar collection (dispatched from handle_calendar_collection)

**Flow:**
1. Extract authenticated user and HrefContext from request extensions
2. Parse request body for property updates
3. **Decision: calendars::get_calendar_by_id() succeeds?**
   - **NO → Terminal: 404 NOT_FOUND** "Calendar not found"
   - **YES → Continue**
4. Extract optional properties from PROPPATCH body:
   - displayname
   - calendar-description
   - calendar-color
5. **Decision: calendars::update_calendar() succeeds?**
   - **NO → Terminal: 500 INTERNAL_SERVER_ERROR** "Failed to update properties"
   - **YES → Continue**
6. Build multistatus response:
   - Determine href based on context (email or username)
   - Add successful properties to 200 propstat
7. **Terminal: 207 MULTI_STATUS** with updated property statuses

---

## 17. Calendar PROPFIND (`PROPFIND /{username}/{calendar_id}/`)

**Entry Point:** PROPFIND method on calendar collection (dispatched from handle_calendar_collection)

**Flow:**
1. Extract authenticated user and HrefContext from request extensions
2. Extract Depth header (0 or 1)
3. Parse PROPFIND request body
4. **Decision: calendars::get_calendar_by_id() succeeds?**
   - **NO → Terminal: 404 NOT_FOUND** "Calendar not found"
   - **YES → Continue**
5. Build multistatus response with context-aware href construction
6. Filter properties against calendar_props_for_context
7. Add calendar resource to response
8. **Decision: Depth >= 1?**
   - **NO → Terminal: 207 MULTI_STATUS** (just calendar)
   - **YES → Continue**
9. Query events::list_objects()
10. For each calendar object: filter properties, add to response with context-aware hrefs
11. **Terminal: 207 MULTI_STATUS** (calendar + objects)

---

## 18. REPORT (`REPORT /{username}/{calendar_id}/`)

**Entry Point:** REPORT method on calendar collection (dispatched from handle_calendar_collection)

**Flow (main dispatch):**
1. Extract authenticated user and HrefContext from request extensions
2. Extract request body (up to 256KB)
3. Build context from HrefContext or create default
4. **Decision: parse_report() succeeds?**
   - **NO → Terminal: 400 BAD_REQUEST** "Invalid REPORT body"
   - **YES → Continue**
5. **Decision: Which REPORT type?**
   - **CalendarMultiget → Call handle_multiget()**
   - **CalendarQuery → Call handle_query()**
   - **SyncCollection → Call handle_sync()**
6. **Terminal: 207 MULTI_STATUS** (from specific handler)

### 18a. calendar-multiget REPORT

**Flow:**
1. Build multistatus response
2. For each href in request:
   - Extract filename (last path segment)
   - Strip .ics extension
   - Percent-decode UID
3. Query events::get_objects_by_uids()
4. For each returned object: build context-aware href, add to response (include_data=true)
5. **Terminal: 207 MULTI_STATUS** with requested objects

### 18b. calendar-query REPORT

**Flow:**
1. Build multistatus response
2. **Decision: time_range filter provided?**
   - **YES → Query events::list_objects_in_range()**
   - **NO → Query events::list_objects()**
3. For each returned object: build context-aware href, add to response (include_data=true)
4. **Terminal: 207 MULTI_STATUS** with filtered objects

### 18c. sync-collection REPORT (RFC 6578)

**Flow:**
1. **Decision: calendars::get_calendar_by_id() succeeds?**
   - **NO → Terminal: 404 NOT_FOUND** "Calendar not found"
   - **YES → Continue**
2. Extract props to determine if client requested calendar-data
3. Build multistatus response
4. **Decision: sync_token is empty?**
   - **YES → Initial sync (full):**
     - Query events::list_objects()
     - For each object: add to response (include_data if requested)
   - **NO → Delta sync:**
     - Query events::get_sync_changes_since()
     - For each change:
       - **Decision: change_type == "deleted"?**
         - **YES → Add as 404 response (deletion marker)**
         - **NO → Fetch current object and add to response**
5. Get current sync token from calendar
6. Ensure token is valid URI format
7. Add sync token to response
8. **Terminal: 207 MULTI_STATUS** with changes/full list + sync token

---

## 19. Email Calendar Collection (`/calendar/dav/{email}/user/{calendar_id}/`)

**Entry Point:** Any method on `/calendar/dav/{email}/user/{calendar_id}/`

**Flow:**
1. Log request (method, URI, email, calendar_id)
2. **Decision: Is method OPTIONS?**
   - **YES → Terminal: 200 OK** OPTIONS response
   - **NO → Continue**
3. Extract auth header
4. **Decision: auth_or_email_user() succeeds?**
   - **NO → Terminal: 401 UNAUTHORIZED**
   - **YES → Continue**
5. **Decision: Method is MKCALENDAR?**
   - **YES → Skip access check** → Continue to method dispatch
   - **NO → Verify calendar access** → Continue
6. **Verify calendar access:**
   - **Decision: User has access to calendar?**
     - **NO → Terminal: 403 FORBIDDEN**
     - **YES → Continue**
7. Insert user and HrefContext into request extensions
8. **Method dispatch:**
   - **PROPFIND → Call handle_calendar() → Terminal: 207 MULTI_STATUS**
   - **REPORT → Call handle_report() → Terminal: 207 MULTI_STATUS**
   - **PROPPATCH → Call handle_proppatch() → Terminal: 207 MULTI_STATUS**
   - **MKCALENDAR → Call handle_mkcalendar() → Terminal: 201 CREATED**
   - **DELETE → Call handle_delete_calendar() → Terminal: 204 NO_CONTENT**
   - **Other → Terminal: 405 METHOD_NOT_ALLOWED**

---

## 20. Email Object (`/calendar/dav/{email}/user/{calendar_id}/{filename}`)

**Entry Point:** Any method on `/calendar/dav/{email}/user/{calendar_id}/{filename}`

**Flow:**
1. Log request (method, URI, email, calendar_id, filename)
2. **Decision: Is method OPTIONS?**
   - **YES → Terminal: 200 OK** OPTIONS response
   - **NO → Continue**
3. Extract auth header
4. **Decision: auth_or_email_user() succeeds?**
   - **NO → Terminal: 401 UNAUTHORIZED**
   - **YES → Continue**
5. **Decision: User has access to calendar?**
   - **NO → Terminal: 403 FORBIDDEN**
   - **YES → Continue**
6. Insert user and HrefContext into request extensions
7. **Method dispatch:**
   - **GET → Call handle_get() → Terminal: 200 OK with iCalendar data**
   - **PUT → Call handle_put() → Terminal: 201 CREATED or 204 NO_CONTENT**
   - **DELETE → Call handle_delete_object() → Terminal: 204 NO_CONTENT**
   - **Other → Terminal: 405 METHOD_NOT_ALLOWED**

---

## 21. MCP Request Flow (Port 5233)

**Entry Point:** Any HTTP request on MCP port 5233

**Flow:**
1. **Decision: Bearer token present in Authorization header?**
   - **NO → Terminal: 401 UNAUTHORIZED**
   - **YES → Continue**
2. Hash token with Argon2id
3. **Decision: Token valid (exists in mcp_tokens, not expired)?**
   - **NO → Terminal: 401 UNAUTHORIZED**
   - **YES → Continue**
4. **Decision: HTTP method?**
   - **GET → Terminal: 200 OK** (Streamable HTTP SSE endpoint)
   - **DELETE → Close session → Terminal: 200 OK**
   - **POST → Continue to JSON-RPC processing**
   - **Other → Terminal: 405 METHOD_NOT_ALLOWED**
5. Parse JSON-RPC 2.0 request
6. **Decision: JSON-RPC method?**
   - **initialize → Terminal: JSON-RPC result** (protocol info, tool list)
   - **ping → Terminal: JSON-RPC result** (empty object)
   - **notifications/initialized → Terminal: 202 Accepted** (notification, no response)
   - **tools/list → Terminal: JSON-RPC result** (tool definitions)
   - **tools/call → Continue to tool dispatch**
   - **Unknown → Terminal: JSON-RPC error** (-32601 Method not found)
7. Extract tool name from params
8. **Decision: Tool dispatch** (12 full-mode tools or 3 simple-mode tools):
   - Calendar tools: list_calendars, get_calendar, create_calendar, delete_calendar
   - Event tools: create_event, get_event, update_event, delete_event, query_events
   - Sharing tools: share_calendar, unshare_calendar, list_shared_calendars
   - Simple mode: add, delete, list
9. Execute tool with user_id from token
10. **Decision: Tool succeeded?**
    - **YES → Terminal: JSON-RPC result** (tool output)
    - **NO → Terminal: JSON-RPC result** (isError=true, error message)

---

## 22. Authentication Strategies

Three authentication strategies used across the server:

### 22a. inline_auth (CalDAV strict)

**Entry Point:** Called from handlers that require authentication

**Flow:**
1. **Decision: Authorization header present?**
   - **NO → Terminal: 401 UNAUTHORIZED**
   - **YES → Continue**
2. Decode HTTP Basic Auth (base64)
3. **Decision: User found by username?**
   - **NO → Terminal: 401 UNAUTHORIZED**
   - **YES → Continue**
4. **Decision: Argon2id password verification passes?**
   - **NO → Terminal: 401 UNAUTHORIZED**
   - **YES → Terminal: Return User** (authenticated)

### 22b. auth_or_path_user (CalDAV flexible)

**Entry Point:** Called from /caldav/users/{username}/* handlers

**Flow:**
1. **Decision: Authorization header present?**
   - **YES → Continue to Basic Auth**
   - **NO → Continue to path fallback**
2. **Basic Auth path:**
   - Decode HTTP Basic Auth
   - **Decision: User found and password valid?**
     - **YES → Terminal: Return User** (authenticated)
     - **NO → Terminal: 401 UNAUTHORIZED**
3. **Path fallback:**
   - Extract {username} from URL path
   - **Decision: User found by username?**
     - **YES → Terminal: Return User** (path-based, no password check)
     - **NO → Terminal: 401 UNAUTHORIZED**

### 22c. require_bearer_auth (MCP middleware)

**Entry Point:** All requests to MCP port 5233

**Flow:**
1. **Decision: Authorization header present with "Bearer " prefix?**
   - **NO → Terminal: 401 UNAUTHORIZED**
   - **YES → Continue**
2. Extract token string
3. Hash token with Argon2id
4. **Decision: Token hash matches any mcp_tokens row?**
   - **NO → Terminal: 401 UNAUTHORIZED**
   - **YES → Continue**
5. **Decision: Token expired?**
   - **YES → Terminal: 401 UNAUTHORIZED**
   - **NO → Continue**
6. Inject user_id into request extensions
7. **Terminal: Continue to handler** (authenticated)

---

## 23. Apple Calendar Discovery and Sync Flow

This is a sequence/protocol diagram rather than a code flowchart. It shows the actual macOS 15.x Apple Calendar two-daemon discovery and sync sequence.

### Phase 1: accountsd (account setup)

1. accountsd sends PROPFIND `/.well-known/caldav` (no auth)
2. Server returns 301 redirect to `/caldav/`
3. accountsd sends PROPFIND `/caldav/` (no auth)
4. Server returns 207 with current-user-principal
5. accountsd sends PROPFIND `/` (no auth)
6. Server returns 207
7. accountsd sends PROPFIND `/principals/` (no auth)
8. Server returns 207
9. accountsd sends PROPFIND `/calendar/dav/{email}/user/` (no auth)
10. Server returns 207 with generic "CalDAV Account" (anti-enumeration)
11. User enters credentials in System Preferences
12. accountsd sends PROPFIND `/calendar/dav/{email}/user/` (with Basic Auth)
13. **Decision: Credentials valid?**
    - **YES → Server returns 207 with username and calendar list**
    - **NO → Server returns 401 Unauthorized**

### Phase 2: dataaccessd (ongoing sync)

1. dataaccessd sends PROPFIND `/calendar/dav/{email}/user/` (Depth:1, with auth)
2. Server returns 207 with email-based calendar hrefs
3. dataaccessd sends PROPFIND on each calendar (email-based paths)
4. Server returns 207 with calendar properties
5. dataaccessd sends REPORT (sync-collection) on each calendar
6. Server returns 207 with events and sync-token
7. dataaccessd sends PROPPATCH to update calendar properties
8. dataaccessd sends PUT/DELETE for event changes
9. dataaccessd periodically re-syncs via REPORT (delta sync with token)

### Security annotations:
- **No-auth discovery**: accountsd probes without credentials, server returns safe structural data
- **Anti-enumeration**: Same 207 response for valid and invalid emails
- **Auth quirk**: dataaccessd only sends credentials to the email discovery URL
- **Path fallback**: /caldav/users/* uses path-based user resolution (no credentials needed)
- **No cookies**: dataaccessd ignores Set-Cookie headers

---

## Terminal Status Summary

| Handler | Success | Error 4xx | Error 5xx |
|---------|---------|-----------|-----------|
| OPTIONS | 200 OK | - | - |
| well-known (non-OPTIONS) | 301 MOVED_PERMANENTLY | - | - |
| PROPFIND (all) | 207 MULTI_STATUS | 401, 404, 405 | 500 |
| GET | 200 OK | 401, 403, 404 | 500 |
| PUT | 201 CREATED or 204 NO_CONTENT | 400, 401, 403, 412 | 500 |
| DELETE (object) | 204 NO_CONTENT | 401, 403, 404 | 500 |
| DELETE (calendar) | 204 NO_CONTENT | 401, 403, 404 | 500 |
| MKCALENDAR | 201 CREATED | 401, 403, 405 | 500 |
| PROPPATCH | 207 MULTI_STATUS | 401, 403, 404 | 500 |
| REPORT | 207 MULTI_STATUS | 400, 401, 403, 404 | 500 |
| MCP (tools/call) | JSON-RPC result | 401 | JSON-RPC error |
