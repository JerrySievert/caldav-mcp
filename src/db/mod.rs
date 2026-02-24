pub mod calendars;
pub mod events;
pub mod models;
pub mod shares;
pub mod tokens;
pub mod users;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

/// Initialize the database connection pool and run migrations.
pub async fn init_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    run_migrations(&pool).await?;

    Ok(pool)
}

/// Run SQL migrations from the migrations directory.
async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let sql = include_str!("../../migrations/001_initial.sql");

    // sqlx::query().execute() only runs the first statement.
    // Split on semicolons and execute each statement individually.
    for statement in sql.split(';') {
        let trimmed = statement.trim();
        // Skip empty segments. Don't skip comments — they may precede
        // SQL in the same segment, and SQLite handles `--` comments fine.
        if trimmed.is_empty() {
            continue;
        }
        // Skip segments that are only comments (no actual SQL).
        let has_sql = trimmed.lines().any(|line| {
            let l = line.trim();
            !l.is_empty() && !l.starts_with("--")
        });
        if !has_sql {
            continue;
        }
        sqlx::query(trimmed).execute(pool).await?;
    }

    Ok(())
}

/// Create an in-memory pool for testing.
#[cfg(test)]
pub async fn test_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("Failed to create test pool");

    // Enable foreign keys for in-memory DB
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("Failed to enable foreign keys");

    run_migrations(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}
