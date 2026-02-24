# caldav-mcp

A CalDAV server with an integrated MCP (Model Context Protocol) interface, written in Rust.

Designed for self-hosting. Compatible with Apple Calendar (macOS/iOS), supports multiple authenticated users, shared calendars, and LLM access via MCP tools on a separate HTTP port.

## Features

- **Full CalDAV support** -- PROPFIND, PROPPATCH, MKCALENDAR, REPORT, PUT, GET, DELETE
- **Apple Calendar compatible** -- tested discovery flow: `.well-known/caldav` -> principal -> calendar home -> calendars -> events
- **Multi-user** -- HTTP Basic Auth with Argon2id password hashing
- **Shared calendars** -- read or read-write permissions between users
- **Efficient sync** -- `calendar-multiget`, `calendar-query` (time-range), and `sync-collection` (RFC 6578 delta sync) REPORT handlers
- **MCP server** -- 12 tools for LLM calendar management, secured with bearer token auth on a separate port
- **SQLite** -- single-file database, WAL mode, zero external dependencies
- **CLI** -- built-in commands for user and token management

## Architecture

Two HTTP servers run in one binary:

| Server | Default Port | Auth | Purpose |
|--------|-------------|------|---------|
| CalDAV | 5232 | HTTP Basic Auth | Calendar clients (Apple Calendar, Thunderbird, etc.) |
| MCP | 5233 | Bearer token | LLM access via Model Context Protocol |

Both share the same SQLite database.

### URL Hierarchy (CalDAV)

```
/.well-known/caldav                                  -> 301 redirect
/caldav/                                             -> root (current-user-principal)
/caldav/principals/{username}/                       -> user principal (calendar-home-set)
/caldav/users/{username}/                            -> calendar home (list calendars)
/caldav/users/{username}/{calendar-id}/              -> calendar collection (PROPFIND, REPORT)
/caldav/users/{username}/{calendar-id}/{uid}.ics     -> individual event (GET, PUT, DELETE)
```

## Getting Started

### Prerequisites

- Rust 1.85+ (edition 2024)

### Build

```bash
git clone https://github.com/jerrysievert/caldav-mcp.git
cd caldav-mcp
cargo build --release
```

### Configure

```bash
cp .env.example .env
```

Edit `.env` as needed:

```bash
# CalDAV server port
CALDAV_PORT=5232

# MCP server port
MCP_PORT=5233

# SQLite database path (created automatically)
DATABASE_URL=sqlite:data/caldav.db?mode=rwc

# Log level (trace, debug, info, warn, error)
RUST_LOG=caldav_server=info,tower_http=info
```

### Create Users and Tokens

```bash
# Create a user
cargo run --release -- create-user -u alice -p secretpassword -e alice@example.com

# Create an MCP token for that user
cargo run --release -- create-token -u alice -n "claude-access"
# Save the printed token -- it cannot be retrieved again

# List users
cargo run --release -- list-users

# List tokens for a user
cargo run --release -- list-tokens -u alice

# Delete a token by ID
cargo run --release -- delete-token -i <token-id>
```

### Start the Server

```bash
cargo run --release

# Or with the explicit serve subcommand
cargo run --release -- serve
```

The database and `data/` directory are created automatically on first run.

## Apple Calendar Setup

1. Open **System Settings** -> **Internet Accounts** -> **Add Account** -> **Other** -> **CalDAV Account**
2. Select **Manual** configuration
3. Enter:
   - **Username**: your username (e.g. `alice`)
   - **Password**: your password
   - **Server Address**: `http://your-server:5232`
4. Calendars should appear automatically

To create a new calendar, use either Apple Calendar's UI or the MCP `create_calendar` tool.

## MCP Integration

The MCP server implements the [Model Context Protocol](https://modelcontextprotocol.io/) using Streamable HTTP transport (JSON-RPC 2.0 over `POST /mcp`).

### Available Tools

#### Calendars
| Tool | Description |
|------|-------------|
| `list_calendars` | List all calendars accessible to the authenticated user (owned + shared) |
| `get_calendar` | Get details about a specific calendar |
| `create_calendar` | Create a new calendar |
| `delete_calendar` | Delete a calendar and all its events |

#### Events
| Tool | Description |
|------|-------------|
| `create_event` | Create a new calendar event |
| `get_event` | Get a specific event by its UID |
| `update_event` | Update an existing event |
| `delete_event` | Delete a calendar event |
| `query_events` | Query events, optionally filtered by time range |

#### Sharing
| Tool | Description |
|------|-------------|
| `share_calendar` | Share a calendar with another user (read or read-write) |
| `unshare_calendar` | Revoke a user's access to a shared calendar |
| `list_shared_calendars` | List calendars shared with the authenticated user |

### Testing with curl

```bash
# Initialize MCP session
curl -X POST http://localhost:5233/mcp \
  -H "Authorization: Bearer mcp_your_token_here" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
      "protocolVersion": "2025-03-26",
      "capabilities": {},
      "clientInfo": {"name": "test", "version": "1.0"}
    }
  }'

# List tools
curl -X POST http://localhost:5233/mcp \
  -H "Authorization: Bearer mcp_your_token_here" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc": "2.0", "id": 2, "method": "tools/list"}'

# List calendars
curl -X POST http://localhost:5233/mcp \
  -H "Authorization: Bearer mcp_your_token_here" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 3,
    "method": "tools/call",
    "params": {
      "name": "list_calendars",
      "arguments": {}
    }
  }'

# Create an event
curl -X POST http://localhost:5233/mcp \
  -H "Authorization: Bearer mcp_your_token_here" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 4,
    "method": "tools/call",
    "params": {
      "name": "create_event",
      "arguments": {
        "calendar_id": "your-calendar-id",
        "title": "Team Standup",
        "start": "20260301T090000Z",
        "end": "20260301T093000Z",
        "description": "Daily sync",
        "location": "Conference Room A"
      }
    }
  }'
```

### Claude Desktop Configuration

Add to your Claude Desktop `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "caldav": {
      "url": "http://localhost:5233/mcp",
      "headers": {
        "Authorization": "Bearer mcp_your_token_here"
      }
    }
  }
}
```

## Shared Calendars

Calendars can be shared between users with either `read` or `read-write` permissions. Shared calendars appear alongside owned calendars in both CalDAV clients and MCP tool responses.

Sharing is managed through the MCP `share_calendar` / `unshare_calendar` tools, or directly in the database.

## Deployment

### Behind a Reverse Proxy (recommended)

For internet exposure, run behind nginx or caddy with TLS:

```nginx
# nginx example
server {
    listen 443 ssl;
    server_name caldav.example.com;

    # CalDAV
    location / {
        proxy_pass http://127.0.0.1:5232;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}

server {
    listen 443 ssl;
    server_name mcp.example.com;

    # MCP -- consider restricting access (VPN, IP allowlist, etc.)
    location / {
        proxy_pass http://127.0.0.1:5233;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

### Security Notes

- Passwords are hashed with Argon2id
- MCP tokens are hashed with Argon2id (only shown once at creation)
- The MCP port should be firewalled or restricted -- it does not need to be publicly accessible unless you specifically need remote LLM access
- Always use TLS in production (via reverse proxy)

## Project Structure

```
src/
  main.rs              -- CLI + dual server startup
  config.rs            -- environment variable config
  error.rs             -- error types
  db/                  -- SQLite data layer
    models.rs          -- User, Calendar, CalendarObject, etc.
    users.rs           -- user CRUD + password verification
    calendars.rs       -- calendar CRUD + ctag/sync-token
    events.rs          -- event CRUD + etag + sync change log
    shares.rs          -- calendar sharing
    tokens.rs          -- MCP token CRUD
  caldav/              -- CalDAV protocol handlers
    auth.rs            -- HTTP Basic Auth middleware
    wellknown.rs       -- /.well-known/caldav + OPTIONS
    propfind.rs        -- PROPFIND at each URL depth
    proppatch.rs       -- PROPPATCH for calendar properties
    mkcalendar.rs      -- MKCALENDAR handler
    get.rs             -- GET .ics resources
    put.rs             -- PUT create/update events
    delete.rs          -- DELETE events and calendars
    report.rs          -- REPORT dispatcher (multiget, query, sync)
    xml/               -- XML parsing and generation
  mcp/                 -- MCP protocol server
    auth.rs            -- Bearer token middleware
    transport.rs       -- Streamable HTTP (POST/GET/DELETE /mcp)
    handlers.rs        -- JSON-RPC dispatch
    jsonrpc.rs         -- JSON-RPC 2.0 types
    session.rs         -- session management
    tools/             -- MCP tool implementations
  ical/                -- iCalendar utilities
    parser.rs          -- extract fields from raw .ics
    builder.rs         -- generate VCALENDAR/VEVENT
migrations/
  001_initial.sql      -- database schema
```

## Running Tests

```bash
cargo test
```

60 unit tests covering the database layer, XML parsing, iCalendar handling, JSON-RPC types, and session management.

## Tech Stack

| Component | Crate |
|-----------|-------|
| Web framework | axum 0.8 |
| Database | sqlx 0.8 (SQLite) |
| XML | quick-xml 0.37 |
| iCalendar | icalendar 0.16 |
| Password hashing | argon2 0.5 |
| CLI | clap 4 |
| Logging | tracing + tower-http |
| Serialization | serde + serde_json |

## License

BSD 3-Clause License. See [LICENSE](LICENSE).
