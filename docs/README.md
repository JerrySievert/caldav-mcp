# CalDAV Server Documentation

A self-hosted CalDAV server with MCP (Model Context Protocol) integration, built in Rust. Designed for personal and small-team calendar management with full Apple Calendar (macOS/iOS) compatibility.

## Table of Contents

### Architecture & Design

- [Architecture Overview](architecture.md) - System architecture, component layout, and design decisions
- [Database Schema](database.md) - Tables, relationships, indexes, and data model

### API References

- [CalDAV API Reference](caldav-api.md) - Full CalDAV protocol implementation (RFC 4791)
- [MCP API Reference](mcp-api.md) - Model Context Protocol tools and JSON-RPC interface

### Security & Authentication

- [Authentication & Security](authentication.md) - Auth strategies, security model, and threat mitigations

### Operations

- [Configuration](configuration.md) - Environment variables and server configuration
- [CLI Reference](cli.md) - Command-line interface for user and token management

### Integration Guides

- [Apple Calendar Integration](apple-calendar.md) - macOS/iOS Calendar.app setup and compatibility details

## Diagrams

All diagrams are in [`docs/images/`](images/) as SVG files. See the [images README](images/README.md) for the full index.

| Diagram | Description |
|---------|-------------|
| [Database Schema](images/schema.svg) | Entity-relationship diagram for all 6 tables |
| [Authentication Strategies](images/flow-authentication.svg) | All 3 auth strategies side by side |
| [Well-Known & Discovery](images/flow-wellknown-discovery.svg) | `/.well-known/caldav` and root discovery endpoints |
| [PROPFIND](images/flow-propfind.svg) | Property find across all 5 URL levels |
| [PUT Event](images/flow-put-event.svg) | Create/update events with ETag and If-Match support |
| [GET Event](images/flow-get-event.svg) | Retrieve calendar objects |
| [DELETE](images/flow-delete.svg) | Delete objects and calendars |
| [MKCALENDAR](images/flow-mkcalendar.svg) | Calendar creation |
| [PROPPATCH](images/flow-proppatch.svg) | Calendar property updates |
| [REPORT](images/flow-report.svg) | calendar-multiget, calendar-query, and sync-collection |
| [MCP Request](images/flow-mcp-request.svg) | MCP JSON-RPC request handling on port 5233 |

## Quick Start

```bash
# Build
cargo build --release

# Create initial user
./target/release/caldav-server create-user --username alice --password secret123

# Create MCP token for AI tool access
./target/release/caldav-server create-token --username alice --name "my-ai-tool"

# Start server (CalDAV on :5232, MCP on :5233)
./target/release/caldav-server serve
```

## URL Hierarchy

```
/.well-known/caldav                                    -> 301 to /caldav/
/caldav/                                               -> Root (PROPFIND: current-user-principal)
/caldav/principals/{username}/                         -> User principal (redirect to calendar home)
/caldav/users/{username}/                              -> Calendar home (PROPFIND: list calendars)
/caldav/users/{username}/{calendar-id}/                -> Calendar collection (PROPFIND/REPORT)
/caldav/users/{username}/{calendar-id}/{uid}.ics       -> Calendar object (GET/PUT/DELETE)
/calendar/dav/{email}/user/                            -> Apple email discovery endpoint
```

## Ports

| Port | Protocol | Auth Method | Purpose |
|------|----------|-------------|---------|
| 5232 | CalDAV (HTTP) | HTTP Basic Auth | Calendar client access |
| 5233 | MCP (HTTP) | Bearer Token | AI tool / programmatic access |

## Standards Compliance

- [RFC 4791](https://datatracker.ietf.org/doc/html/rfc4791) - CalDAV (Calendaring Extensions to WebDAV)
- [RFC 5545](https://datatracker.ietf.org/doc/html/rfc5545) - iCalendar (Internet Calendaring and Scheduling)
- [RFC 6578](https://datatracker.ietf.org/doc/html/rfc6578) - Collection Synchronization for WebDAV
- [RFC 6764](https://datatracker.ietf.org/doc/html/rfc6764) - Locating CalDAV Services (Well-Known URI)
- [MCP 2025-03-26](https://modelcontextprotocol.io/) - Model Context Protocol (Streamable HTTP transport)
