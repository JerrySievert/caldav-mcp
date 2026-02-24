# Database Schema

![Database Schema](images/database-schema.svg)

The CalDAV server uses SQLite with WAL (Write-Ahead Logging) journal mode. The database file is stored at the path specified by `DATABASE_URL` (default: `data/caldav.db`).

## Overview

| Table | Purpose | Relationships |
|-------|---------|---------------|
| `users` | User accounts with hashed passwords | Root entity |
| `calendars` | Calendar collections with metadata | Owned by users |
| `calendar_objects` | Events/todos stored as raw iCalendar | Belong to calendars |
| `calendar_shares` | Sharing permissions between users | Links calendars to users |
| `sync_changes` | Change log for delta sync (RFC 6578) | References calendars |
| `mcp_tokens` | API tokens for MCP access | Owned by users |

## Tables

### users

Stores user accounts. Passwords are hashed with Argon2id (OWASP-recommended algorithm) using a per-password salt generated with `OsRng`.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | TEXT | PRIMARY KEY | UUID v7 (time-sortable) |
| `username` | TEXT | UNIQUE, NOT NULL | Login identifier |
| `email` | TEXT | UNIQUE | Optional email (used for Apple Calendar email discovery) |
| `password_hash` | TEXT | NOT NULL | Argon2id hash with embedded salt |
| `created_at` | TIMESTAMP | NOT NULL, DEFAULT CURRENT_TIMESTAMP | Account creation time |

**Usage:**
- Username is the primary login credential for HTTP Basic Auth
- Email is used by Apple Calendar's `/calendar/dav/{email}/user/` discovery endpoint
- The same user can authenticate via CalDAV (password) or MCP (bearer token)

### calendars

Calendar collections that contain events. Each calendar has a `ctag` (change tag) that increments on any mutation, and a `sync_token` for RFC 6578 delta synchronization.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | TEXT | PRIMARY KEY | UUID v7 |
| `owner_id` | TEXT | FK -> users.id, NOT NULL | Calendar owner |
| `name` | TEXT | NOT NULL | Display name |
| `description` | TEXT | | Optional description |
| `color` | TEXT | | Hex color (e.g., `#FF5733`) |
| `timezone` | TEXT | | IANA timezone (default: UTC) |
| `ctag` | TEXT | | Change tag - changes on any calendar mutation |
| `sync_token` | TEXT | | Current sync token (UUID v7 format: `sync-{uuid}`) |
| `created_at` | TIMESTAMP | NOT NULL, DEFAULT CURRENT_TIMESTAMP | Creation time |
| `updated_at` | TIMESTAMP | NOT NULL, DEFAULT CURRENT_TIMESTAMP | Last modification |

**Key behaviors:**
- `ctag` is bumped on every PUT, DELETE, or PROPPATCH affecting the calendar or its objects
- `sync_token` is regenerated (new UUID v7) on every object mutation for delta sync
- Deleting a calendar cascades to all `calendar_objects`, `calendar_shares`, and `sync_changes`

### calendar_objects

Individual calendar objects (events, todos). Stores the full raw iCalendar text along with extracted indexed fields for query performance.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | TEXT | PRIMARY KEY | UUID v7 |
| `calendar_id` | TEXT | FK -> calendars.id, NOT NULL | Parent calendar |
| `uid` | TEXT | NOT NULL | iCalendar UID (unique within calendar) |
| `etag` | TEXT | NOT NULL | ETag for conditional requests (UUID v4 format, quoted) |
| `ical_data` | TEXT | NOT NULL | Full raw .ics file content |
| `component_type` | TEXT | | VEVENT, VTODO, etc. |
| `dtstart` | TEXT | | Start date/time (extracted, indexed for range queries) |
| `dtend` | TEXT | | End date/time (extracted, indexed for range queries) |
| `summary` | TEXT | | Event title (extracted for search) |
| `created_at` | TIMESTAMP | NOT NULL, DEFAULT CURRENT_TIMESTAMP | Object creation |
| `updated_at` | TIMESTAMP | NOT NULL, DEFAULT CURRENT_TIMESTAMP | Last modification |

**Key behaviors:**
- `ical_data` stores the complete .ics text as received from the client
- `dtstart`, `dtend`, `summary` are extracted during PUT for indexed queries
- `etag` is regenerated (new UUID v4) on every update
- Time-range queries use: `dtstart < end AND dtend > start`
- For VTODOs, `DUE` is used instead of `DTEND`
- iCal line unfolding handles both `\r\n ` and `\n ` continuation patterns (RFC 5545)

### calendar_shares

Manages calendar sharing between users. Each share grants a specific permission level.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | TEXT | PRIMARY KEY | UUID v7 |
| `calendar_id` | TEXT | FK -> calendars.id, NOT NULL | Shared calendar |
| `user_id` | TEXT | FK -> users.id, NOT NULL | Recipient user |
| `permission` | TEXT | NOT NULL | `"read"` or `"read-write"` |
| `created_at` | TIMESTAMP | NOT NULL, DEFAULT CURRENT_TIMESTAMP | Share creation |

**Constraints:**
- `UNIQUE(calendar_id, user_id)` - One share per user per calendar
- Upsert behavior: sharing again updates the permission level

**Permission levels:**

| Permission | Can Read | Can Create/Update Events | Can Delete Calendar |
|-----------|----------|--------------------------|---------------------|
| `read` | Yes | No | No |
| `read-write` | Yes | Yes | No |
| (owner) | Yes | Yes | Yes |

### sync_changes

Change log for RFC 6578 sync-collection REPORT. Every object mutation (create, modify, delete) is logged with the current sync token.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | INTEGER | PRIMARY KEY AUTOINCREMENT | Sequential ID |
| `calendar_id` | TEXT | FK -> calendars.id, NOT NULL | Affected calendar |
| `object_uid` | TEXT | NOT NULL | Event UID (not FK - may reference deleted objects) |
| `change_type` | TEXT | NOT NULL | `"created"`, `"modified"`, or `"deleted"` |
| `sync_token` | TEXT | NOT NULL | Token at time of change |
| `created_at` | TIMESTAMP | NOT NULL, DEFAULT CURRENT_TIMESTAMP | Change timestamp |

**Key behaviors:**
- `object_uid` is not a foreign key because deleted objects no longer exist in `calendar_objects`
- Delta sync queries: `WHERE calendar_id = ? AND sync_token > ?` (ordered by id)
- For deleted objects, the sync-collection REPORT returns a 404 status for that href
- Full sync (empty token) returns all current objects instead of querying this table

### mcp_tokens

Bearer tokens for MCP API authentication. Tokens are hashed with Argon2id before storage.

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | TEXT | PRIMARY KEY | UUID v7 |
| `user_id` | TEXT | FK -> users.id, NOT NULL | Token owner |
| `token_hash` | TEXT | NOT NULL | Argon2id hash of the raw token |
| `name` | TEXT | NOT NULL | Human-readable token name (for management) |
| `created_at` | TIMESTAMP | NOT NULL, DEFAULT CURRENT_TIMESTAMP | Token creation |
| `expires_at` | TIMESTAMP | | Optional expiration date |

**Key behaviors:**
- Raw token format: `mcp_{base64-url-safe-32-bytes}`
- The raw token is shown once at creation time (via CLI) and never stored
- Token validation iterates all tokens and checks each hash (Argon2id verification is timing-safe)
- Expired tokens are not automatically cleaned up (manual deletion via CLI)

## Entity Relationships

```
users (1) ──────< (many) calendars
  │                    │
  │                    ├──< (many) calendar_objects
  │                    │
  │                    ├──< (many) calendar_shares
  │                    │
  │                    └──< (many) sync_changes
  │
  ├──────< (many) calendar_shares
  │
  └──────< (many) mcp_tokens
```

## Indexes

The following indexes are used for query performance:

| Table | Columns | Purpose |
|-------|---------|---------|
| `calendar_objects` | `(calendar_id, uid)` | Primary lookup for events |
| `calendar_objects` | `(dtstart, dtend)` | Time-range queries (calendar-query REPORT) |
| `calendar_shares` | `(user_id)` | List shared calendars for a user |
| `calendar_shares` | `(calendar_id, user_id)` | Unique constraint + lookup |
| `sync_changes` | `(calendar_id, sync_token)` | Delta sync queries |

## Migration

The schema is defined in `migrations/001_initial.sql` and applied automatically on server startup. The migration runner checks for already-applied migrations and only runs new ones.

## SQLite Configuration

- **Journal mode:** WAL (Write-Ahead Logging) for concurrent reads
- **Foreign keys:** Enabled (`PRAGMA foreign_keys = ON`)
- **Connection pool:** Managed by sqlx with async access
- **File path:** Configurable via `DATABASE_URL` environment variable
