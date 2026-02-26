mod caldav;
mod config;
mod db;
mod error;
mod ical;
mod mcp;

use std::net::SocketAddr;

use clap::{Parser, Subcommand};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "caldav-server", about = "CalDAV server with MCP interface")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the CalDAV + MCP servers (default)
    Serve,

    /// Create a new user
    CreateUser {
        /// Username
        #[arg(short, long)]
        username: String,
        /// Password
        #[arg(short, long)]
        password: String,
        /// Email address (optional)
        #[arg(short, long)]
        email: Option<String>,
    },

    /// Create an MCP API token for a user
    CreateToken {
        /// Username of the token owner
        #[arg(short, long)]
        username: String,
        /// A descriptive name for this token
        #[arg(short, long)]
        name: String,
    },

    /// List all users
    ListUsers,

    /// List MCP tokens for a user
    ListTokens {
        /// Username
        #[arg(short, long)]
        username: String,
    },

    /// Delete an MCP token by ID
    DeleteToken {
        /// Token ID to delete
        #[arg(short, long)]
        id: String,
    },

    /// Reset a user's password
    ResetPassword {
        /// Username
        #[arg(short, long)]
        username: String,
        /// New password
        #[arg(short, long)]
        password: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file if present
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();

    // Commands that don't need full server tracing
    let command = cli.command.unwrap_or(Commands::Serve);

    match command {
        Commands::Serve => run_server().await,
        Commands::CreateUser {
            username,
            password,
            email,
        } => cmd_create_user(&username, &password, email.as_deref()).await,
        Commands::CreateToken { username, name } => cmd_create_token(&username, &name).await,
        Commands::ListUsers => cmd_list_users().await,
        Commands::ListTokens { username } => cmd_list_tokens(&username).await,
        Commands::DeleteToken { id } => cmd_delete_token(&id).await,
        Commands::ResetPassword { username, password } => {
            cmd_reset_password(&username, &password).await
        }
    }
}

/// Start both the CalDAV and MCP servers.
async fn run_server() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = config::Config::from_env()?;
    tracing::info!(
        caldav_port = config.caldav_port,
        mcp_port = config.mcp_port,
        "Starting CalDAV server"
    );

    let pool = db::init_pool(&config.database_url).await?;
    tracing::info!("Database initialized");

    let caldav_app = caldav::router(pool.clone());
    tracing::info!(tool_mode = %config.tool_mode, "MCP tool mode");
    let mcp_app = mcp::router(pool.clone(), config.tool_mode.clone());

    let caldav_addr = SocketAddr::from(([0, 0, 0, 0], config.caldav_port));
    let caldav_listener = TcpListener::bind(caldav_addr).await?;
    tracing::info!(%caldav_addr, "CalDAV server listening");

    let mcp_addr = SocketAddr::from(([0, 0, 0, 0], config.mcp_port));
    let mcp_listener = TcpListener::bind(mcp_addr).await?;
    tracing::info!(%mcp_addr, "MCP server listening");

    tokio::try_join!(
        axum::serve(caldav_listener, caldav_app).into_future(),
        axum::serve(mcp_listener, mcp_app).into_future(),
    )?;

    Ok(())
}

/// Helper: init a DB pool from env for CLI commands.
async fn cli_pool() -> anyhow::Result<sqlx::SqlitePool> {
    let config = config::Config::from_env()?;
    Ok(db::init_pool(&config.database_url).await?)
}

/// Create a new user.
async fn cmd_create_user(
    username: &str,
    password: &str,
    email: Option<&str>,
) -> anyhow::Result<()> {
    let pool = cli_pool().await?;
    let user = db::users::create_user(&pool, username, email, password).await?;
    println!("User created:");
    println!("  ID:       {}", user.id);
    println!("  Username: {}", user.username);
    if let Some(ref e) = user.email {
        println!("  Email:    {e}");
    }
    Ok(())
}

/// Create an MCP token for a user.
async fn cmd_create_token(username: &str, name: &str) -> anyhow::Result<()> {
    let pool = cli_pool().await?;
    let user = db::users::get_user_by_username(&pool, username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("User '{username}' not found"))?;

    let (raw_token, record) = db::tokens::create_token(&pool, &user.id, name).await?;
    println!("MCP token created:");
    println!("  ID:    {}", record.id);
    println!("  Name:  {}", record.name);
    println!("  Token: {raw_token}");
    println!();
    println!("Save this token — it cannot be retrieved again.");
    Ok(())
}

/// List all users.
async fn cmd_list_users() -> anyhow::Result<()> {
    let pool = cli_pool().await?;
    let users = sqlx::query_as::<_, db::models::User>("SELECT * FROM users ORDER BY username")
        .fetch_all(&pool)
        .await?;

    if users.is_empty() {
        println!("No users found.");
        return Ok(());
    }

    println!("{:<38} {:<20} Email", "ID", "Username");
    println!("{}", "-".repeat(70));
    for u in &users {
        println!(
            "{:<38} {:<20} {}",
            u.id,
            u.username,
            u.email.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

/// List MCP tokens for a user.
async fn cmd_list_tokens(username: &str) -> anyhow::Result<()> {
    let pool = cli_pool().await?;
    let user = db::users::get_user_by_username(&pool, username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("User '{username}' not found"))?;

    let tokens = db::tokens::list_tokens_for_user(&pool, &user.id).await?;
    if tokens.is_empty() {
        println!("No tokens found for user '{username}'.");
        return Ok(());
    }

    println!("{:<38} {:<20} Created", "ID", "Name");
    println!("{}", "-".repeat(70));
    for t in &tokens {
        println!("{:<38} {:<20} {}", t.id, t.name, t.created_at);
    }
    Ok(())
}

/// Delete an MCP token by ID.
async fn cmd_delete_token(token_id: &str) -> anyhow::Result<()> {
    let pool = cli_pool().await?;
    db::tokens::delete_token(&pool, token_id).await?;
    println!("Token {token_id} deleted.");
    Ok(())
}

/// Reset a user's password.
async fn cmd_reset_password(username: &str, password: &str) -> anyhow::Result<()> {
    let pool = cli_pool().await?;
    db::users::reset_password(&pool, username, password).await?;
    println!("Password updated for user '{username}'.");
    Ok(())
}
