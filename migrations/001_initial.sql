-- Users
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    username TEXT UNIQUE NOT NULL,
    email TEXT,
    password_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Calendars
CREATE TABLE IF NOT EXISTS calendars (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id),
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    color TEXT NOT NULL DEFAULT '#0E61B9',
    timezone TEXT NOT NULL DEFAULT 'UTC',
    ctag TEXT NOT NULL,
    sync_token TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Calendar objects (events, todos)
CREATE TABLE IF NOT EXISTS calendar_objects (
    id TEXT PRIMARY KEY,
    calendar_id TEXT NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    uid TEXT NOT NULL,
    etag TEXT NOT NULL,
    ical_data TEXT NOT NULL,
    component_type TEXT NOT NULL DEFAULT 'VEVENT',
    dtstart TEXT,
    dtend TEXT,
    summary TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(calendar_id, uid)
);

-- Calendar sharing
CREATE TABLE IF NOT EXISTS calendar_shares (
    id TEXT PRIMARY KEY,
    calendar_id TEXT NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    permission TEXT NOT NULL CHECK (permission IN ('read', 'read-write')),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(calendar_id, user_id)
);

-- Sync change log (for sync-collection REPORT)
CREATE TABLE IF NOT EXISTS sync_changes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    calendar_id TEXT NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    object_uid TEXT NOT NULL,
    change_type TEXT NOT NULL CHECK (change_type IN ('created', 'modified', 'deleted')),
    sync_token TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- MCP API tokens
CREATE TABLE IF NOT EXISTS mcp_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_calendar_objects_calendar_id ON calendar_objects(calendar_id);
CREATE INDEX IF NOT EXISTS idx_calendar_objects_uid ON calendar_objects(uid);
CREATE INDEX IF NOT EXISTS idx_calendar_objects_dtstart ON calendar_objects(dtstart);
CREATE INDEX IF NOT EXISTS idx_calendar_objects_dtend ON calendar_objects(dtend);
CREATE INDEX IF NOT EXISTS idx_calendar_shares_user_id ON calendar_shares(user_id);
CREATE INDEX IF NOT EXISTS idx_calendar_shares_calendar_id ON calendar_shares(calendar_id);
CREATE INDEX IF NOT EXISTS idx_sync_changes_calendar_id_token ON sync_changes(calendar_id, sync_token);
