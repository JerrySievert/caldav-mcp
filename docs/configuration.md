# Configuration

The CalDAV server is configured entirely through environment variables. There are no configuration files.

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CALDAV_PORT` | `5232` | Port for the CalDAV HTTP server |
| `MCP_PORT` | `5233` | Port for the MCP HTTP server |
| `DATABASE_URL` | `sqlite:data/caldav.db?mode=rwc` | SQLite connection string |
| `RUST_LOG` | (unset) | Logging level for tracing |

### CALDAV_PORT

The TCP port where the CalDAV server listens for HTTP connections. CalDAV clients (Apple Calendar, Thunderbird, etc.) connect to this port.

```bash
CALDAV_PORT=5232  # default
CALDAV_PORT=8080  # custom
```

**Standard CalDAV port:** The CalDAV community convention is port 5232 (used by Radicale and others). Using the default ensures compatibility with clients that auto-discover on this port.

### MCP_PORT

The TCP port where the MCP (Model Context Protocol) server listens. AI tools and automation clients connect to this port.

```bash
MCP_PORT=5233  # default
MCP_PORT=8081  # custom
```

### DATABASE_URL

SQLite connection string in sqlx format. The `mode=rwc` flag enables read-write-create mode (creates the database file if it doesn't exist).

```bash
DATABASE_URL="sqlite:data/caldav.db?mode=rwc"     # default (relative path)
DATABASE_URL="sqlite:/var/lib/caldav/caldav.db?mode=rwc"  # absolute path
```

**Notes:**
- The directory must exist (the file is created automatically)
- WAL journal mode is enabled automatically for concurrent read performance
- Foreign key enforcement is enabled on every connection

### RUST_LOG

Controls the verbosity of structured logging via the `tracing` crate and `tracing-subscriber`.

```bash
RUST_LOG=info                    # General info-level logging
RUST_LOG=debug                   # Verbose debug output
RUST_LOG=caldav_server=debug     # Debug only for this crate
RUST_LOG=tower_http=debug        # Debug HTTP request/response logging
RUST_LOG=sqlx=warn               # Reduce SQL query noise
```

**Recommended for development:**
```bash
RUST_LOG=debug
```

**Recommended for production:**
```bash
RUST_LOG=info
```

## Using a .env File

The server reads environment variables from the process environment. You can use a `.env` file with a tool like `dotenv` or `direnv`, or source it before starting:

```bash
# .env
CALDAV_PORT=5232
MCP_PORT=5233
DATABASE_URL="sqlite:data/caldav.db?mode=rwc"
RUST_LOG=info
```

```bash
# Source and run
source .env && caldav-server serve

# Or with direnv (automatic)
# Add .env to .envrc
```

## Deployment Examples

### Development (Local)

```bash
RUST_LOG=debug caldav-server serve
```

Both servers start on localhost:5232 and localhost:5233.

### Production (Behind Reverse Proxy)

```bash
CALDAV_PORT=5232 \
MCP_PORT=5233 \
DATABASE_URL="sqlite:/var/lib/caldav/caldav.db?mode=rwc" \
RUST_LOG=info \
caldav-server serve
```

**Nginx configuration:**
```nginx
server {
    listen 443 ssl;
    server_name caldav.example.com;

    ssl_certificate /etc/ssl/certs/caldav.pem;
    ssl_certificate_key /etc/ssl/private/caldav.key;

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

    ssl_certificate /etc/ssl/certs/mcp.pem;
    ssl_certificate_key /etc/ssl/private/mcp.key;

    # MCP
    location / {
        proxy_pass http://127.0.0.1:5233;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

**Caddy configuration:**
```
caldav.example.com {
    reverse_proxy localhost:5232
}

mcp.example.com {
    reverse_proxy localhost:5233
}
```

### Docker

```dockerfile
FROM rust:1.83 AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/caldav-server /usr/local/bin/
VOLUME /data
ENV DATABASE_URL="sqlite:/data/caldav.db?mode=rwc"
EXPOSE 5232 5233
CMD ["caldav-server", "serve"]
```

```bash
docker run -d \
  -p 5232:5232 \
  -p 5233:5233 \
  -v caldav-data:/data \
  caldav-server
```

### systemd Service

```ini
[Unit]
Description=CalDAV Server
After=network.target

[Service]
Type=simple
User=caldav
Group=caldav
WorkingDirectory=/opt/caldav
Environment=CALDAV_PORT=5232
Environment=MCP_PORT=5233
Environment=DATABASE_URL=sqlite:/var/lib/caldav/caldav.db?mode=rwc
Environment=RUST_LOG=info
ExecStart=/opt/caldav/caldav-server serve
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```
