# Stage 1: Build
FROM rust:1.85-bookworm AS builder

WORKDIR /app

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs so cargo can fetch + compile dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release && rm -rf src target/release/deps/caldav_server*

# Copy actual source and rebuild (only the crate, not deps)
COPY src/ src/
COPY migrations/ migrations/
RUN cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/caldav-server /usr/local/bin/caldav-server
COPY migrations/ /app/migrations/

WORKDIR /app

# Create data directory for SQLite (host can bind-mount over this)
RUN mkdir -p /data

# Defaults — overridden by .env bind-mount or docker-compose environment
ENV DATABASE_URL="sqlite:/data/caldav.db?mode=rwc"
ENV CALDAV_PORT=5232
ENV MCP_PORT=5233
ENV RUST_LOG=info
ENV MCP_TOOL_MODE=simple

EXPOSE 5232 5233

ENTRYPOINT ["caldav-server"]
CMD ["serve"]
