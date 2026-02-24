# CLI Reference

The CalDAV server binary provides several commands for server operation and user/token management. All commands share the same database connection configured via `DATABASE_URL`.

## Usage

```bash
caldav-server <COMMAND> [OPTIONS]
```

## Commands

### serve

Starts both the CalDAV and MCP HTTP servers.

```bash
caldav-server serve
```

This is the default command. It:
1. Initializes the SQLite database and runs pending migrations
2. Starts the CalDAV server on `CALDAV_PORT` (default: 5232)
3. Starts the MCP server on `MCP_PORT` (default: 5233)
4. Enables request logging via tower-http TraceLayer

Both servers run concurrently and share the same database connection pool.

**Example:**
```bash
# Start with default settings
caldav-server serve

# Start with custom ports and logging
CALDAV_PORT=8080 MCP_PORT=8081 RUST_LOG=debug caldav-server serve
```

### create-user

Creates a new user account.

```bash
caldav-server create-user --username <USERNAME> --password <PASSWORD> [--email <EMAIL>]
```

| Option | Required | Description |
|--------|----------|-------------|
| `--username` | Yes | Unique login name |
| `--password` | Yes | Password (hashed with Argon2id before storage) |
| `--email` | No | Email address (used for Apple Calendar email discovery) |

**Example:**
```bash
# Basic user
caldav-server create-user --username alice --password 'my-secure-password'

# User with email (needed for Apple Calendar)
caldav-server create-user --username alice --password 'my-secure-password' --email alice@example.com
```

**Output:**
```
Created user alice (id: 01234567-89ab-cdef-0123-456789abcdef)
```

**Notes:**
- Username must be unique (returns error if already exists)
- Email must be unique if provided
- Password is hashed with Argon2id using a random salt
- The email is required if you want Apple Calendar's email-based discovery to work

### create-token

Creates a new MCP bearer token for a user.

```bash
caldav-server create-token --username <USERNAME> --name <TOKEN_NAME>
```

| Option | Required | Description |
|--------|----------|-------------|
| `--username` | Yes | User to create the token for |
| `--name` | Yes | Human-readable name for the token |

**Example:**
```bash
caldav-server create-token --username alice --name "claude-code"
```

**Output:**
```
Created token for alice:
  ID: 01234567-89ab-cdef-0123-456789abcdef
  Token: mcp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef01234567

Save this token - it cannot be retrieved later.
```

**Notes:**
- The raw token (`mcp_...`) is displayed once and never stored (only the Argon2id hash is saved)
- Copy the token immediately - there is no way to retrieve it later
- Use this token in the `Authorization: Bearer <token>` header for MCP requests

### list-users

Lists all registered users.

```bash
caldav-server list-users
```

**Output:**
```
Users:
  alice (id: 01234567-89ab-cdef-0123-456789abcdef)
  bob   (id: fedcba98-7654-3210-fedc-ba9876543210)
```

### list-tokens

Lists all MCP tokens for a specific user.

```bash
caldav-server list-tokens --username <USERNAME>
```

| Option | Required | Description |
|--------|----------|-------------|
| `--username` | Yes | User whose tokens to list |

**Output:**
```
Tokens for alice:
  claude-code    (id: 01234567-89ab-cdef-0123-456789abcdef, created: 2026-02-20)
  automation     (id: fedcba98-7654-3210-fedc-ba9876543210, created: 2026-02-21)
```

**Notes:**
- Only shows token metadata (ID, name, creation date) - not the raw token value
- Token hash is never displayed

### delete-token

Deletes an MCP token by its ID.

```bash
caldav-server delete-token --id <TOKEN_ID>
```

| Option | Required | Description |
|--------|----------|-------------|
| `--id` | Yes | Token UUID to delete |

**Example:**
```bash
caldav-server delete-token --id 01234567-89ab-cdef-0123-456789abcdef
```

**Output:**
```
Deleted token 01234567-89ab-cdef-0123-456789abcdef
```

### reset-password

Resets a user's password.

```bash
caldav-server reset-password --username <USERNAME> --password <NEW_PASSWORD>
```

| Option | Required | Description |
|--------|----------|-------------|
| `--username` | Yes | User whose password to reset |
| `--password` | Yes | New password (hashed with Argon2id) |

**Example:**
```bash
caldav-server reset-password --username alice --password 'new-secure-password'
```

**Output:**
```
Password reset for alice
```

## Common Workflows

### Initial Setup

```bash
# 1. Create a user with email (for Apple Calendar)
caldav-server create-user \
  --username alice \
  --password 'strong-password-here' \
  --email alice@example.com

# 2. Create an MCP token for AI tool access
caldav-server create-token --username alice --name "my-ai-tool"
# Save the displayed token!

# 3. Start the server
caldav-server serve
```

### Rotating an MCP Token

```bash
# 1. List current tokens
caldav-server list-tokens --username alice

# 2. Delete the old token
caldav-server delete-token --id <old-token-id>

# 3. Create a new token
caldav-server create-token --username alice --name "my-ai-tool-v2"
# Update your client with the new token
```

### Adding a Second User

```bash
# Create user
caldav-server create-user --username bob --password 'bobs-password' --email bob@example.com

# Optionally create an MCP token
caldav-server create-token --username bob --name "bobs-tool"
```
