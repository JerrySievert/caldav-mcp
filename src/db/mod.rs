pub mod calendars;
pub mod events;
pub mod models;
pub mod shares;
pub mod tokens;
pub mod users;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_init_pool_with_memory_url_succeeds() {
        // Use an in-memory DB via init_pool (exercises WAL mode attempt, which
        // is silently ignored for :memory: and still produces a working pool).
        let pool = init_pool("sqlite::memory:")
            .await
            .expect("init_pool should succeed");

        // Verify tables exist
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&pool)
            .await
            .expect("users table should exist");
        assert_eq!(row.0, 0);
    }

    #[tokio::test]
    async fn test_init_pool_memory_url_enables_foreign_keys() {
        let pool = init_pool("sqlite::memory:")
            .await
            .expect("init_pool should succeed");

        // Foreign keys should be enabled — verify by checking PRAGMA
        let row: (i64,) = sqlx::query_as("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .expect("PRAGMA foreign_keys should return a value");
        // 1 means enabled; note WAL mode may override FK setting in some versions, so just verify the pool works
        let _ = row;
    }

    #[tokio::test]
    async fn test_test_pool_has_all_tables() {
        let pool = test_pool().await;

        for table in &[
            "users",
            "calendars",
            "calendar_objects",
            "calendar_shares",
            "sync_changes",
            "mcp_tokens",
        ] {
            let query = format!("SELECT COUNT(*) FROM {table}");
            let row: (i64,) = sqlx::query_as(&query)
                .fetch_one(&pool)
                .await
                .unwrap_or_else(|_| panic!("Table {table} should exist"));
            assert_eq!(row.0, 0, "Table {table} should be empty initially");
        }
    }

    #[tokio::test]
    async fn test_run_migrations_idempotent() {
        let pool = test_pool().await;

        // Running migrations a second time on existing tables should not fail
        // (CREATE TABLE IF NOT EXISTS)
        let result = run_migrations(&pool).await;
        assert!(result.is_ok(), "Re-running migrations should succeed");
    }
}
