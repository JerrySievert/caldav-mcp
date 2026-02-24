use sqlx::SqlitePool;
use uuid::Uuid;

use super::models::{Calendar, CalendarShare, Permission};
use crate::error::{AppError, AppResult};

/// Share a calendar with a user at a given permission level.
pub async fn share_calendar(
    pool: &SqlitePool,
    calendar_id: &str,
    user_id: &str,
    permission: Permission,
) -> AppResult<CalendarShare> {
    let id = Uuid::now_v7().to_string();

    sqlx::query(
        "INSERT INTO calendar_shares (id, calendar_id, user_id, permission)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(calendar_id, user_id) DO UPDATE SET permission = excluded.permission",
    )
    .bind(&id)
    .bind(calendar_id)
    .bind(user_id)
    .bind(permission.as_str())
    .execute(pool)
    .await?;

    let share = sqlx::query_as::<_, CalendarShare>(
        "SELECT * FROM calendar_shares WHERE calendar_id = ? AND user_id = ?",
    )
    .bind(calendar_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    Ok(share)
}

/// Revoke a user's access to a calendar.
pub async fn unshare_calendar(
    pool: &SqlitePool,
    calendar_id: &str,
    user_id: &str,
) -> AppResult<()> {
    let result = sqlx::query(
        "DELETE FROM calendar_shares WHERE calendar_id = ? AND user_id = ?",
    )
    .bind(calendar_id)
    .bind(user_id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(
            "Share not found".to_string(),
        ));
    }
    Ok(())
}

/// List all shares for a calendar.
#[allow(dead_code)]
pub async fn list_shares_for_calendar(
    pool: &SqlitePool,
    calendar_id: &str,
) -> AppResult<Vec<CalendarShare>> {
    let shares = sqlx::query_as::<_, CalendarShare>(
        "SELECT * FROM calendar_shares WHERE calendar_id = ?",
    )
    .bind(calendar_id)
    .fetch_all(pool)
    .await?;
    Ok(shares)
}

/// List all calendars shared with a user.
pub async fn list_shared_calendars(
    pool: &SqlitePool,
    user_id: &str,
) -> AppResult<Vec<(Calendar, Permission)>> {
    // First get the shares for this user
    let shares: Vec<CalendarShare> = sqlx::query_as(
        "SELECT * FROM calendar_shares WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut results = Vec::new();
    for share in shares {
        if let Some(cal) = super::calendars::get_calendar_by_id(pool, &share.calendar_id).await? {
            if let Some(perm) = Permission::from_str_value(&share.permission) {
                results.push((cal, perm));
            }
        }
    }

    // Sort by name for consistent ordering
    results.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    Ok(results)
}

/// Check what permission a user has on a calendar (owner = ReadWrite, shared, or None).
#[allow(dead_code)]
pub async fn get_user_permission(
    pool: &SqlitePool,
    calendar_id: &str,
    user_id: &str,
) -> AppResult<Option<Permission>> {
    // Check if user owns the calendar
    let is_owner: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM calendars WHERE id = ? AND owner_id = ?",
    )
    .bind(calendar_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    if is_owner.is_some() {
        return Ok(Some(Permission::ReadWrite));
    }

    // Check shares
    let share: Option<(String,)> = sqlx::query_as(
        "SELECT permission FROM calendar_shares WHERE calendar_id = ? AND user_id = ?",
    )
    .bind(calendar_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(share.and_then(|(p,)| Permission::from_str_value(&p)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::db::{calendars, users};

    async fn setup() -> (SqlitePool, String, String, String) {
        let pool = db::test_pool().await;
        let alice = users::create_user(&pool, "alice", None, "pass").await.unwrap();
        let bob = users::create_user(&pool, "bob", None, "pass").await.unwrap();
        let cal = calendars::create_calendar(&pool, &alice.id, "Work", "", "#FF0000", "UTC")
            .await
            .unwrap();
        (pool, alice.id, bob.id, cal.id)
    }

    #[tokio::test]
    async fn test_share_calendar() {
        let (pool, _, bob_id, cal_id) = setup().await;

        let share = share_calendar(&pool, &cal_id, &bob_id, Permission::Read)
            .await
            .unwrap();

        assert_eq!(share.calendar_id, cal_id);
        assert_eq!(share.user_id, bob_id);
        assert_eq!(share.permission, "read");
    }

    #[tokio::test]
    async fn test_update_share_permission() {
        let (pool, _, bob_id, cal_id) = setup().await;

        share_calendar(&pool, &cal_id, &bob_id, Permission::Read).await.unwrap();
        let updated = share_calendar(&pool, &cal_id, &bob_id, Permission::ReadWrite).await.unwrap();

        assert_eq!(updated.permission, "read-write");
    }

    #[tokio::test]
    async fn test_unshare_calendar() {
        let (pool, _, bob_id, cal_id) = setup().await;

        share_calendar(&pool, &cal_id, &bob_id, Permission::Read).await.unwrap();
        unshare_calendar(&pool, &cal_id, &bob_id).await.unwrap();

        let shares = list_shares_for_calendar(&pool, &cal_id).await.unwrap();
        assert!(shares.is_empty());
    }

    #[tokio::test]
    async fn test_unshare_nonexistent() {
        let (pool, _, bob_id, cal_id) = setup().await;

        let result = unshare_calendar(&pool, &cal_id, &bob_id).await;
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_owner_has_read_write() {
        let (pool, alice_id, _, cal_id) = setup().await;

        let perm = get_user_permission(&pool, &cal_id, &alice_id).await.unwrap();
        assert_eq!(perm, Some(Permission::ReadWrite));
    }

    #[tokio::test]
    async fn test_shared_user_permission() {
        let (pool, _, bob_id, cal_id) = setup().await;

        share_calendar(&pool, &cal_id, &bob_id, Permission::Read).await.unwrap();
        let perm = get_user_permission(&pool, &cal_id, &bob_id).await.unwrap();
        assert_eq!(perm, Some(Permission::Read));
    }

    #[tokio::test]
    async fn test_no_permission() {
        let (pool, _, bob_id, cal_id) = setup().await;

        let perm = get_user_permission(&pool, &cal_id, &bob_id).await.unwrap();
        assert_eq!(perm, None);
    }

    #[tokio::test]
    async fn test_list_shared_calendars_for_user() {
        let (pool, alice_id, bob_id, cal_id) = setup().await;

        // Share alice's calendar with bob
        share_calendar(&pool, &cal_id, &bob_id, Permission::Read).await.unwrap();

        // Bob's shared calendars should include alice's
        let shared = list_shared_calendars(&pool, &bob_id).await.unwrap();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].0.id, cal_id);
        assert_eq!(shared[0].1, Permission::Read);

        // Alice shouldn't see anything in shared (she's the owner)
        let alice_shared = list_shared_calendars(&pool, &alice_id).await.unwrap();
        assert!(alice_shared.is_empty());
    }

    #[tokio::test]
    async fn test_shared_calendars_appear_in_list_for_user() {
        let (pool, _, bob_id, cal_id) = setup().await;

        share_calendar(&pool, &cal_id, &bob_id, Permission::Read).await.unwrap();

        let all_cals = calendars::list_calendars_for_user(&pool, &bob_id).await.unwrap();
        assert_eq!(all_cals.len(), 1);
        assert_eq!(all_cals[0].id, cal_id);
    }
}
