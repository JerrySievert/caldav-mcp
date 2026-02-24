use sqlx::SqlitePool;
use uuid::Uuid;

use super::models::Calendar;
use crate::error::{AppError, AppResult};

/// Generate a new sync token (monotonically increasing UUID v7).
fn new_sync_token() -> String {
    format!("sync-{}", Uuid::now_v7())
}

/// Create a new calendar for a user. Returns the created calendar.
pub async fn create_calendar(
    pool: &SqlitePool,
    owner_id: &str,
    name: &str,
    description: &str,
    color: &str,
    timezone: &str,
) -> AppResult<Calendar> {
    let id = Uuid::now_v7().to_string();
    create_calendar_with_id(pool, &id, owner_id, name, description, color, timezone).await
}

/// Create a new calendar with a specific ID. Returns the created calendar.
pub async fn create_calendar_with_id(
    pool: &SqlitePool,
    id: &str,
    owner_id: &str,
    name: &str,
    description: &str,
    color: &str,
    timezone: &str,
) -> AppResult<Calendar> {
    let sync_token = new_sync_token();

    sqlx::query(
        "INSERT INTO calendars (id, owner_id, name, description, color, timezone, ctag, sync_token)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(owner_id)
    .bind(name)
    .bind(description)
    .bind(color)
    .bind(timezone)
    .bind(&sync_token)
    .bind(&sync_token)
    .execute(pool)
    .await?;

    get_calendar_by_id(pool, &id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("Calendar created but not found")))
}

/// Get a calendar by its ID.
pub async fn get_calendar_by_id(pool: &SqlitePool, id: &str) -> AppResult<Option<Calendar>> {
    let cal = sqlx::query_as::<_, Calendar>("SELECT * FROM calendars WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(cal)
}

/// List all calendars owned by a user.
#[allow(dead_code)]
pub async fn list_calendars_for_owner(
    pool: &SqlitePool,
    owner_id: &str,
) -> AppResult<Vec<Calendar>> {
    let cals = sqlx::query_as::<_, Calendar>(
        "SELECT * FROM calendars WHERE owner_id = ? ORDER BY name",
    )
    .bind(owner_id)
    .fetch_all(pool)
    .await?;
    Ok(cals)
}

/// List all calendars accessible to a user (owned + shared with them).
pub async fn list_calendars_for_user(
    pool: &SqlitePool,
    user_id: &str,
) -> AppResult<Vec<Calendar>> {
    let cals = sqlx::query_as::<_, Calendar>(
        "SELECT c.* FROM calendars c WHERE c.owner_id = ?
         UNION
         SELECT c.* FROM calendars c
         INNER JOIN calendar_shares cs ON cs.calendar_id = c.id
         WHERE cs.user_id = ?
         ORDER BY name",
    )
    .bind(user_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(cals)
}

/// Update a calendar's properties. Returns the updated calendar.
pub async fn update_calendar(
    pool: &SqlitePool,
    id: &str,
    name: Option<&str>,
    description: Option<&str>,
    color: Option<&str>,
    timezone: Option<&str>,
) -> AppResult<Calendar> {
    let cal = get_calendar_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Calendar {id} not found")))?;

    let name = name.unwrap_or(&cal.name);
    let description = description.unwrap_or(&cal.description);
    let color = color.unwrap_or(&cal.color);
    let timezone = timezone.unwrap_or(&cal.timezone);

    sqlx::query(
        "UPDATE calendars SET name = ?, description = ?, color = ?, timezone = ?,
         updated_at = datetime('now') WHERE id = ?",
    )
    .bind(name)
    .bind(description)
    .bind(color)
    .bind(timezone)
    .bind(id)
    .execute(pool)
    .await?;

    get_calendar_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("Calendar updated but not found")))
}

/// Delete a calendar and all its objects (cascade).
pub async fn delete_calendar(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM calendars WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Calendar {id} not found")));
    }
    Ok(())
}

/// Bump the ctag and sync_token for a calendar (called after any object mutation).
pub async fn bump_ctag(pool: &SqlitePool, calendar_id: &str) -> AppResult<String> {
    let new_token = new_sync_token();

    sqlx::query(
        "UPDATE calendars SET ctag = ?, sync_token = ?, updated_at = datetime('now')
         WHERE id = ?",
    )
    .bind(&new_token)
    .bind(&new_token)
    .bind(calendar_id)
    .execute(pool)
    .await?;

    Ok(new_token)
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
    async fn test_create_and_get_calendar() {
        let (pool, user_id) = setup().await;

        let cal = create_calendar(&pool, &user_id, "Work", "Work events", "#FF0000", "UTC")
            .await
            .unwrap();

        assert_eq!(cal.name, "Work");
        assert_eq!(cal.description, "Work events");
        assert_eq!(cal.color, "#FF0000");
        assert_eq!(cal.owner_id, user_id);
        assert!(cal.ctag.starts_with("sync-"));

        let fetched = get_calendar_by_id(&pool, &cal.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, cal.id);
    }

    #[tokio::test]
    async fn test_list_calendars_for_owner() {
        let (pool, user_id) = setup().await;

        create_calendar(&pool, &user_id, "Work", "", "#FF0000", "UTC")
            .await
            .unwrap();
        create_calendar(&pool, &user_id, "Personal", "", "#00FF00", "UTC")
            .await
            .unwrap();

        let cals = list_calendars_for_owner(&pool, &user_id).await.unwrap();
        assert_eq!(cals.len(), 2);
        // Sorted by name
        assert_eq!(cals[0].name, "Personal");
        assert_eq!(cals[1].name, "Work");
    }

    #[tokio::test]
    async fn test_update_calendar() {
        let (pool, user_id) = setup().await;

        let cal = create_calendar(&pool, &user_id, "Work", "", "#FF0000", "UTC")
            .await
            .unwrap();

        let updated = update_calendar(&pool, &cal.id, Some("Office"), None, Some("#0000FF"), None)
            .await
            .unwrap();

        assert_eq!(updated.name, "Office");
        assert_eq!(updated.color, "#0000FF");
        assert_eq!(updated.description, ""); // unchanged
    }

    #[tokio::test]
    async fn test_delete_calendar() {
        let (pool, user_id) = setup().await;

        let cal = create_calendar(&pool, &user_id, "Temp", "", "#000", "UTC")
            .await
            .unwrap();

        delete_calendar(&pool, &cal.id).await.unwrap();

        let fetched = get_calendar_by_id(&pool, &cal.id).await.unwrap();
        assert!(fetched.is_none());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_calendar() {
        let (pool, _) = setup().await;

        let result = delete_calendar(&pool, "nonexistent").await;
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_bump_ctag() {
        let (pool, user_id) = setup().await;

        let cal = create_calendar(&pool, &user_id, "Work", "", "#FF0000", "UTC")
            .await
            .unwrap();
        let original_ctag = cal.ctag.clone();

        // Small delay to ensure UUID v7 differs
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;

        let new_token = bump_ctag(&pool, &cal.id).await.unwrap();
        assert_ne!(new_token, original_ctag);

        let updated = get_calendar_by_id(&pool, &cal.id).await.unwrap().unwrap();
        assert_eq!(updated.ctag, new_token);
        assert_eq!(updated.sync_token, new_token);
    }
}
