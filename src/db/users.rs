use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::models::User;
use crate::error::{AppError, AppResult};

/// Create a new user with a hashed password. Returns the created user.
pub async fn create_user(
    pool: &SqlitePool,
    username: &str,
    email: Option<&str>,
    password: &str,
) -> AppResult<User> {
    let id = Uuid::now_v7().to_string();
    let password_hash = hash_password(password)?;

    sqlx::query(
        "INSERT INTO users (id, username, email, password_hash) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(username)
    .bind(email)
    .bind(&password_hash)
    .execute(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(ref db_err) if db_err.message().contains("UNIQUE") => {
            AppError::Conflict(format!("User '{username}' already exists"))
        }
        _ => AppError::Database(e),
    })?;

    get_user_by_username(pool, username)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("User created but not found")))
}

/// Look up a user by username.
pub async fn get_user_by_username(
    pool: &SqlitePool,
    username: &str,
) -> AppResult<Option<User>> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = ?")
        .bind(username)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

/// Look up a user by ID.
#[allow(dead_code)]
pub async fn get_user_by_id(pool: &SqlitePool, id: &str) -> AppResult<Option<User>> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

/// Look up a user by email address.
pub async fn get_user_by_email(pool: &SqlitePool, email: &str) -> AppResult<Option<User>> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = ?")
        .bind(email)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

/// Reset a user's password by hashing the new password and updating the DB.
pub async fn reset_password(
    pool: &SqlitePool,
    username: &str,
    new_password: &str,
) -> AppResult<()> {
    let hash = hash_password(new_password)?;
    let rows = sqlx::query("UPDATE users SET password_hash = ? WHERE username = ?")
        .bind(&hash)
        .bind(username)
        .execute(pool)
        .await?
        .rows_affected();
    if rows == 0 {
        Err(AppError::Internal(anyhow::anyhow!(
            "User '{username}' not found"
        )))
    } else {
        Ok(())
    }
}

/// Verify a password against a user's stored hash. Returns the user if valid.
/// Accepts either username or email as the login identifier.
pub async fn verify_user(
    pool: &SqlitePool,
    username: &str,
    password: &str,
) -> AppResult<Option<User>> {
    // Try by username first, then by email
    let user = match get_user_by_username(pool, username).await? {
        Some(u) => u,
        None => match get_user_by_email(pool, username).await? {
            Some(u) => u,
            None => return Ok(None),
        },
    };

    if verify_password(password, &user.password_hash)? {
        Ok(Some(user))
    } else {
        Ok(None)
    }
}

/// Hash a password using Argon2id.
fn hash_password(password: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Password hashing failed: {e}")))?;
    Ok(hash.to_string())
}

/// Verify a password against a stored hash.
fn verify_password(password: &str, hash: &str) -> AppResult<bool> {
    let parsed_hash = PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Invalid password hash: {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn test_create_and_get_user() {
        let pool = db::test_pool().await;

        let user = create_user(&pool, "alice", Some("alice@example.com"), "password123")
            .await
            .unwrap();

        assert_eq!(user.username, "alice");
        assert_eq!(user.email.as_deref(), Some("alice@example.com"));
        assert!(!user.password_hash.is_empty());

        let fetched = get_user_by_username(&pool, "alice").await.unwrap().unwrap();
        assert_eq!(fetched.id, user.id);
    }

    #[tokio::test]
    async fn test_duplicate_username_fails() {
        let pool = db::test_pool().await;

        create_user(&pool, "alice", None, "pass1").await.unwrap();
        let result = create_user(&pool, "alice", None, "pass2").await;

        assert!(matches!(result, Err(AppError::Conflict(_))));
    }

    #[tokio::test]
    async fn test_verify_correct_password() {
        let pool = db::test_pool().await;

        create_user(&pool, "alice", None, "secret123").await.unwrap();
        let user = verify_user(&pool, "alice", "secret123").await.unwrap();

        assert!(user.is_some());
        assert_eq!(user.unwrap().username, "alice");
    }

    #[tokio::test]
    async fn test_verify_wrong_password() {
        let pool = db::test_pool().await;

        create_user(&pool, "alice", None, "secret123").await.unwrap();
        let user = verify_user(&pool, "alice", "wrong").await.unwrap();

        assert!(user.is_none());
    }

    #[tokio::test]
    async fn test_verify_nonexistent_user() {
        let pool = db::test_pool().await;

        let user = verify_user(&pool, "nobody", "password").await.unwrap();
        assert!(user.is_none());
    }

    #[tokio::test]
    async fn test_get_user_by_id() {
        let pool = db::test_pool().await;

        let created = create_user(&pool, "alice", None, "pass").await.unwrap();
        let fetched = get_user_by_id(&pool, &created.id).await.unwrap().unwrap();

        assert_eq!(fetched.username, "alice");
    }
}
