use sqlx::SqlitePool;
use uuid::Uuid;

use super::models::{CalendarObject, SyncChange};
use crate::error::{AppError, AppResult};

/// Extracted iCalendar fields stored alongside the raw `ical_data`.
pub struct ObjectFields<'a> {
    pub component_type: &'a str,
    pub dtstart: Option<&'a str>,
    pub dtend: Option<&'a str>,
    pub summary: Option<&'a str>,
}

/// Generate a new ETag value.
fn new_etag() -> String {
    format!("\"{}\"", Uuid::new_v4())
}

/// Create or update a calendar object. Returns the object and whether it was created (vs updated).
pub async fn upsert_object(
    pool: &SqlitePool,
    calendar_id: &str,
    uid: &str,
    ical_data: &str,
    fields: ObjectFields<'_>,
) -> AppResult<(CalendarObject, bool)> {
    let ObjectFields {
        component_type,
        dtstart,
        dtend,
        summary,
    } = fields;
    let existing = get_object_by_uid(pool, calendar_id, uid).await?;
    let is_new = existing.is_none();

    let etag = new_etag();
    let new_sync_token = format!("data:,sync-{}", Uuid::now_v7());

    if is_new {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO calendar_objects
             (id, calendar_id, uid, etag, ical_data, component_type, dtstart, dtend, summary)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(calendar_id)
        .bind(uid)
        .bind(&etag)
        .bind(ical_data)
        .bind(component_type)
        .bind(dtstart)
        .bind(dtend)
        .bind(summary)
        .execute(pool)
        .await?;

        // Log sync change
        log_sync_change(pool, calendar_id, uid, "created", &new_sync_token).await?;
    } else {
        sqlx::query(
            "UPDATE calendar_objects SET etag = ?, ical_data = ?, component_type = ?,
             dtstart = ?, dtend = ?, summary = ?, updated_at = datetime('now')
             WHERE calendar_id = ? AND uid = ?",
        )
        .bind(&etag)
        .bind(ical_data)
        .bind(component_type)
        .bind(dtstart)
        .bind(dtend)
        .bind(summary)
        .bind(calendar_id)
        .bind(uid)
        .execute(pool)
        .await?;

        // Log sync change
        log_sync_change(pool, calendar_id, uid, "modified", &new_sync_token).await?;
    }

    // Bump the calendar's ctag and sync_token
    super::calendars::bump_ctag(pool, calendar_id).await?;

    let obj = get_object_by_uid(pool, calendar_id, uid)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("Object upserted but not found")))?;

    Ok((obj, is_new))
}

/// Get a calendar object by its UID within a calendar.
pub async fn get_object_by_uid(
    pool: &SqlitePool,
    calendar_id: &str,
    uid: &str,
) -> AppResult<Option<CalendarObject>> {
    let obj = sqlx::query_as::<_, CalendarObject>(
        "SELECT * FROM calendar_objects WHERE calendar_id = ? AND uid = ?",
    )
    .bind(calendar_id)
    .bind(uid)
    .fetch_optional(pool)
    .await?;
    Ok(obj)
}

/// List all calendar objects in a calendar.
pub async fn list_objects(pool: &SqlitePool, calendar_id: &str) -> AppResult<Vec<CalendarObject>> {
    let objs = sqlx::query_as::<_, CalendarObject>(
        "SELECT * FROM calendar_objects WHERE calendar_id = ? ORDER BY dtstart",
    )
    .bind(calendar_id)
    .fetch_all(pool)
    .await?;
    Ok(objs)
}

/// List calendar objects within a time range.
pub async fn list_objects_in_range(
    pool: &SqlitePool,
    calendar_id: &str,
    start: &str,
    end: &str,
) -> AppResult<Vec<CalendarObject>> {
    let objs = sqlx::query_as::<_, CalendarObject>(
        "SELECT * FROM calendar_objects
         WHERE calendar_id = ?
           AND dtstart IS NOT NULL
           AND dtend IS NOT NULL
           AND dtstart < ?
           AND dtend > ?
         ORDER BY dtstart",
    )
    .bind(calendar_id)
    .bind(end)
    .bind(start)
    .fetch_all(pool)
    .await?;
    Ok(objs)
}

/// Get multiple calendar objects by their UIDs.
pub async fn get_objects_by_uids(
    pool: &SqlitePool,
    calendar_id: &str,
    uids: &[String],
) -> AppResult<Vec<CalendarObject>> {
    if uids.is_empty() {
        return Ok(vec![]);
    }

    // Build a query with IN clause
    let placeholders: Vec<&str> = uids.iter().map(|_| "?").collect();
    let query = format!(
        "SELECT * FROM calendar_objects WHERE calendar_id = ? AND uid IN ({}) ORDER BY dtstart",
        placeholders.join(", ")
    );

    let mut q = sqlx::query_as::<_, CalendarObject>(&query).bind(calendar_id);
    for uid in uids {
        q = q.bind(uid);
    }

    let objs = q.fetch_all(pool).await?;
    Ok(objs)
}

/// Delete a calendar object by UID. Returns the deleted object's ETag.
pub async fn delete_object(pool: &SqlitePool, calendar_id: &str, uid: &str) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM calendar_objects WHERE calendar_id = ? AND uid = ?")
        .bind(calendar_id)
        .bind(uid)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "Object with UID '{uid}' not found in calendar"
        )));
    }

    let new_sync_token = format!("data:,sync-{}", Uuid::now_v7());
    log_sync_change(pool, calendar_id, uid, "deleted", &new_sync_token).await?;
    super::calendars::bump_ctag(pool, calendar_id).await?;

    Ok(())
}

/// Log a sync change for the sync-collection REPORT.
async fn log_sync_change(
    pool: &SqlitePool,
    calendar_id: &str,
    object_uid: &str,
    change_type: &str,
    sync_token: &str,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO sync_changes (calendar_id, object_uid, change_type, sync_token)
         VALUES (?, ?, ?, ?)",
    )
    .bind(calendar_id)
    .bind(object_uid)
    .bind(change_type)
    .bind(sync_token)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get sync changes after a given sync token for a calendar.
pub async fn get_sync_changes_since(
    pool: &SqlitePool,
    calendar_id: &str,
    since_token: &str,
) -> AppResult<Vec<SyncChange>> {
    // Find the ID of the sync change record with this token
    let anchor: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM sync_changes WHERE calendar_id = ? AND sync_token = ? LIMIT 1",
    )
    .bind(calendar_id)
    .bind(since_token)
    .fetch_optional(pool)
    .await?;

    let changes = match anchor {
        Some((anchor_id,)) => {
            sqlx::query_as::<_, SyncChange>(
                "SELECT * FROM sync_changes WHERE calendar_id = ? AND id > ? ORDER BY id",
            )
            .bind(calendar_id)
            .bind(anchor_id)
            .fetch_all(pool)
            .await?
        }
        None => {
            // If token not found, return all changes (full sync)
            sqlx::query_as::<_, SyncChange>(
                "SELECT * FROM sync_changes WHERE calendar_id = ? ORDER BY id",
            )
            .bind(calendar_id)
            .fetch_all(pool)
            .await?
        }
    };

    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::db::{calendars, users};

    async fn setup() -> (SqlitePool, String, String) {
        let pool = db::test_pool().await;
        let user = users::create_user(&pool, "alice", None, "pass")
            .await
            .unwrap();
        let cal = calendars::create_calendar(&pool, &user.id, "Work", "", "#FF0000", "UTC")
            .await
            .unwrap();
        (pool, user.id, cal.id)
    }

    #[tokio::test]
    async fn test_create_object() {
        let (pool, _, cal_id) = setup().await;

        let (obj, is_new) = upsert_object(
            &pool,
            &cal_id,
            "event-1@example.com",
            "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
            ObjectFields {
                component_type: "VEVENT",
                dtstart: Some("20260301T090000Z"),
                dtend: Some("20260301T100000Z"),
                summary: Some("Meeting"),
            },
        )
        .await
        .unwrap();

        assert!(is_new);
        assert_eq!(obj.uid, "event-1@example.com");
        assert_eq!(obj.summary.as_deref(), Some("Meeting"));
        assert!(obj.etag.starts_with('"'));
    }

    #[tokio::test]
    async fn test_update_object() {
        let (pool, _, cal_id) = setup().await;

        let (original, _) = upsert_object(
            &pool,
            &cal_id,
            "event-1@example.com",
            "original data",
            ObjectFields {
                component_type: "VEVENT",
                dtstart: Some("20260301T090000Z"),
                dtend: Some("20260301T100000Z"),
                summary: Some("Meeting"),
            },
        )
        .await
        .unwrap();

        let (updated, is_new) = upsert_object(
            &pool,
            &cal_id,
            "event-1@example.com",
            "updated data",
            ObjectFields {
                component_type: "VEVENT",
                dtstart: Some("20260301T090000Z"),
                dtend: Some("20260301T110000Z"),
                summary: Some("Long Meeting"),
            },
        )
        .await
        .unwrap();

        assert!(!is_new);
        assert_eq!(updated.ical_data, "updated data");
        assert_eq!(updated.summary.as_deref(), Some("Long Meeting"));
        assert_ne!(updated.etag, original.etag);
    }

    #[tokio::test]
    async fn test_list_objects() {
        let (pool, _, cal_id) = setup().await;

        upsert_object(
            &pool,
            &cal_id,
            "e1@ex.com",
            "data1",
            ObjectFields {
                component_type: "VEVENT",
                dtstart: Some("20260301T090000Z"),
                dtend: Some("20260301T100000Z"),
                summary: Some("First"),
            },
        )
        .await
        .unwrap();
        upsert_object(
            &pool,
            &cal_id,
            "e2@ex.com",
            "data2",
            ObjectFields {
                component_type: "VEVENT",
                dtstart: Some("20260302T090000Z"),
                dtend: Some("20260302T100000Z"),
                summary: Some("Second"),
            },
        )
        .await
        .unwrap();

        let objs = list_objects(&pool, &cal_id).await.unwrap();
        assert_eq!(objs.len(), 2);
    }

    #[tokio::test]
    async fn test_list_objects_in_range() {
        let (pool, _, cal_id) = setup().await;

        upsert_object(
            &pool,
            &cal_id,
            "e1@ex.com",
            "data1",
            ObjectFields {
                component_type: "VEVENT",
                dtstart: Some("20260301T090000Z"),
                dtend: Some("20260301T100000Z"),
                summary: Some("March"),
            },
        )
        .await
        .unwrap();
        upsert_object(
            &pool,
            &cal_id,
            "e2@ex.com",
            "data2",
            ObjectFields {
                component_type: "VEVENT",
                dtstart: Some("20260401T090000Z"),
                dtend: Some("20260401T100000Z"),
                summary: Some("April"),
            },
        )
        .await
        .unwrap();

        let objs = list_objects_in_range(&pool, &cal_id, "20260301T000000Z", "20260331T235959Z")
            .await
            .unwrap();

        assert_eq!(objs.len(), 1);
        assert_eq!(objs[0].summary.as_deref(), Some("March"));
    }

    #[tokio::test]
    async fn test_delete_object() {
        let (pool, _, cal_id) = setup().await;

        upsert_object(
            &pool,
            &cal_id,
            "e1@ex.com",
            "data",
            ObjectFields {
                component_type: "VEVENT",
                dtstart: None,
                dtend: None,
                summary: None,
            },
        )
        .await
        .unwrap();

        delete_object(&pool, &cal_id, "e1@ex.com").await.unwrap();

        let obj = get_object_by_uid(&pool, &cal_id, "e1@ex.com")
            .await
            .unwrap();
        assert!(obj.is_none());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_object() {
        let (pool, _, cal_id) = setup().await;

        let result = delete_object(&pool, &cal_id, "nope@ex.com").await;
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_get_objects_by_uids() {
        let (pool, _, cal_id) = setup().await;

        upsert_object(
            &pool,
            &cal_id,
            "e1@ex.com",
            "d1",
            ObjectFields {
                component_type: "VEVENT",
                dtstart: None,
                dtend: None,
                summary: None,
            },
        )
        .await
        .unwrap();
        upsert_object(
            &pool,
            &cal_id,
            "e2@ex.com",
            "d2",
            ObjectFields {
                component_type: "VEVENT",
                dtstart: None,
                dtend: None,
                summary: None,
            },
        )
        .await
        .unwrap();
        upsert_object(
            &pool,
            &cal_id,
            "e3@ex.com",
            "d3",
            ObjectFields {
                component_type: "VEVENT",
                dtstart: None,
                dtend: None,
                summary: None,
            },
        )
        .await
        .unwrap();

        let uids = vec!["e1@ex.com".to_string(), "e3@ex.com".to_string()];
        let objs = get_objects_by_uids(&pool, &cal_id, &uids).await.unwrap();
        assert_eq!(objs.len(), 2);
    }

    #[tokio::test]
    async fn test_sync_changes() {
        let (pool, _, cal_id) = setup().await;

        // Get the initial sync token
        let cal = calendars::get_calendar_by_id(&pool, &cal_id)
            .await
            .unwrap()
            .unwrap();
        let initial_token = cal.sync_token.clone();

        // Make some changes
        upsert_object(
            &pool,
            &cal_id,
            "e1@ex.com",
            "d1",
            ObjectFields {
                component_type: "VEVENT",
                dtstart: None,
                dtend: None,
                summary: None,
            },
        )
        .await
        .unwrap();
        upsert_object(
            &pool,
            &cal_id,
            "e2@ex.com",
            "d2",
            ObjectFields {
                component_type: "VEVENT",
                dtstart: None,
                dtend: None,
                summary: None,
            },
        )
        .await
        .unwrap();

        // Get changes since initial token
        let changes = get_sync_changes_since(&pool, &cal_id, &initial_token)
            .await
            .unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].change_type, "created");
        assert_eq!(changes[1].change_type, "created");
    }

    #[tokio::test]
    async fn test_upsert_bumps_ctag() {
        let (pool, _, cal_id) = setup().await;

        let cal_before = calendars::get_calendar_by_id(&pool, &cal_id)
            .await
            .unwrap()
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(2)).await;

        upsert_object(
            &pool,
            &cal_id,
            "e1@ex.com",
            "d1",
            ObjectFields {
                component_type: "VEVENT",
                dtstart: None,
                dtend: None,
                summary: None,
            },
        )
        .await
        .unwrap();

        let cal_after = calendars::get_calendar_by_id(&pool, &cal_id)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(cal_before.ctag, cal_after.ctag);
    }
}
