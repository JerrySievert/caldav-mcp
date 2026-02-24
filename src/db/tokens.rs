use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use base64::Engine;
use rand::RngCore;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::models::McpToken;
use crate::error::{AppError, AppResult};

/// Create a new MCP token for a user. Returns the raw token (only shown once)
/// and the stored record.
pub async fn create_token(
    pool: &SqlitePool,
    user_id: &str,
    name: &str,
) -> AppResult<(String, McpToken)> {
    let id = Uuid::now_v7().to_string();
    let raw_token = generate_raw_token();
    let token_hash = hash_token(&raw_token)?;

    sqlx::query(
        "INSERT INTO mcp_tokens (id, user_id, token_hash, name) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(&token_hash)
    .bind(name)
    .execute(pool)
    .await?;

    let record = sqlx::query_as::<_, McpToken>("SELECT * FROM mcp_tokens WHERE id = ?")
        .bind(&id)
        .fetch_one(pool)
        .await?;

    Ok((raw_token, record))
}

/// Validate a raw token and return the associated user ID if valid.
pub async fn validate_token(pool: &SqlitePool, raw_token: &str) -> AppResult<Option<String>> {
    let tokens = sqlx::query_as::<_, McpToken>(
        "SELECT * FROM mcp_tokens WHERE expires_at IS NULL OR expires_at > datetime('now')",
    )
    .fetch_all(pool)
    .await?;

    for token in tokens {
        if verify_token(raw_token, &token.token_hash)? {
            return Ok(Some(token.user_id));
        }
    }

    Ok(None)
}

/// Delete an MCP token by ID.
pub async fn delete_token(pool: &SqlitePool, token_id: &str) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM mcp_tokens WHERE id = ?")
        .bind(token_id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Token not found".to_string()));
    }
    Ok(())
}

/// List all tokens for a user (without raw values).
pub async fn list_tokens_for_user(
    pool: &SqlitePool,
    user_id: &str,
) -> AppResult<Vec<McpToken>> {
    let tokens = sqlx::query_as::<_, McpToken>(
        "SELECT * FROM mcp_tokens WHERE user_id = ? ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(tokens)
}

/// Generate a cryptographically random token string.
fn generate_raw_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!(
        "mcp_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

/// Hash a token using Argon2id.
fn hash_token(token: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(token.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Token hashing failed: {e}")))?;
    Ok(hash.to_string())
}

/// Verify a raw token against a stored hash.
fn verify_token(token: &str, hash: &str) -> AppResult<bool> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Invalid token hash: {e}")))?;
    Ok(Argon2::default()
        .verify_password(token.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::db::users;

    async fn setup() -> (SqlitePool, String) {
        let pool = db::test_pool().await;
        let user = users::create_user(&pool, "alice", None, "pass")
            .await
            .unwrap();
        (pool, user.id)
    }

    #[tokio::test]
    async fn test_create_and_validate_token() {
        let (pool, user_id) = setup().await;

        let (raw_token, record) = create_token(&pool, &user_id, "test-token")
            .await
            .unwrap();

        assert!(raw_token.starts_with("mcp_"));
        assert_eq!(record.name, "test-token");
        assert_eq!(record.user_id, user_id);

        // Validate the token
        let validated_user = validate_token(&pool, &raw_token).await.unwrap();
        assert_eq!(validated_user, Some(user_id));
    }

    #[tokio::test]
    async fn test_invalid_token() {
        let (pool, user_id) = setup().await;

        create_token(&pool, &user_id, "test").await.unwrap();

        let result = validate_token(&pool, "mcp_invalid_token").await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_delete_token() {
        let (pool, user_id) = setup().await;

        let (raw_token, record) = create_token(&pool, &user_id, "test").await.unwrap();
        delete_token(&pool, &record.id).await.unwrap();

        let result = validate_token(&pool, &raw_token).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_list_tokens() {
        let (pool, user_id) = setup().await;

        create_token(&pool, &user_id, "token-1").await.unwrap();
        create_token(&pool, &user_id, "token-2").await.unwrap();

        let tokens = list_tokens_for_user(&pool, &user_id).await.unwrap();
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].name, "token-1");
        assert_eq!(tokens[1].name, "token-2");
    }
}
