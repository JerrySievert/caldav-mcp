# Diagrams

SVG diagrams for the CalDAV server. Flow diagrams are generated from the flow data in [`docs/flow.md`](../flow.md). The schema diagram is generated from [`migrations/001_initial.sql`](../../migrations/001_initial.sql).

## Flow Diagrams

| Diagram | File | Description |
|---------|------|-------------|
| Authentication | [flow-authentication.svg](flow-authentication.svg) | All 3 auth strategies: `inline_auth`, `auth_or_path_user`, `require_bearer_auth` |
| Well-Known & Discovery | [flow-wellknown-discovery.svg](flow-wellknown-discovery.svg) | `/.well-known/caldav` and root discovery endpoints |
| PROPFIND | [flow-propfind.svg](flow-propfind.svg) | Property find across all 5 URL levels |
| PUT Event | [flow-put-event.svg](flow-put-event.svg) | Create/update events with ETag and If-Match support |
| GET Event | [flow-get-event.svg](flow-get-event.svg) | Retrieve calendar objects |
| DELETE | [flow-delete.svg](flow-delete.svg) | Delete objects and calendars |
| MKCALENDAR | [flow-mkcalendar.svg](flow-mkcalendar.svg) | Calendar creation |
| PROPPATCH | [flow-proppatch.svg](flow-proppatch.svg) | Calendar property updates |
| REPORT | [flow-report.svg](flow-report.svg) | calendar-multiget, calendar-query, and sync-collection |
| MCP Request | [flow-mcp-request.svg](flow-mcp-request.svg) | MCP JSON-RPC request handling on port 5233 |

## Schema Diagram

| Diagram | File | Description |
|---------|------|-------------|
| Database Schema | [schema.svg](schema.svg) | Entity-relationship diagram for all 6 tables |

## Generation

Flow diagrams are generated using the skill reference at [`skills/diagrams/references/flowchart.md`](../../skills/diagrams/references/flowchart.md). The schema diagram uses [`skills/diagrams/references/schema.md`](../../skills/diagrams/references/schema.md).
