use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// A registered user.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub password_hash: String,
    pub created_at: NaiveDateTime,
}

/// A calendar collection owned by a user.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Calendar {
    pub id: String,
    pub owner_id: String,
    pub name: String,
    pub description: String,
    pub color: String,
    pub timezone: String,
    pub ctag: String,
    pub sync_token: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// A calendar object (VEVENT, VTODO, etc.) stored as raw iCalendar data.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CalendarObject {
    pub id: String,
    pub calendar_id: String,
    pub uid: String,
    pub etag: String,
    pub ical_data: String,
    pub component_type: String,
    pub dtstart: Option<String>,
    pub dtend: Option<String>,
    pub summary: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// A sharing grant giving a user access to another user's calendar.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CalendarShare {
    pub id: String,
    pub calendar_id: String,
    pub user_id: String,
    pub permission: String,
    pub created_at: NaiveDateTime,
}

/// A record in the sync change log for sync-collection REPORT.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SyncChange {
    pub id: i64,
    pub calendar_id: String,
    pub object_uid: String,
    pub change_type: String,
    pub sync_token: String,
    pub created_at: NaiveDateTime,
}

/// An MCP API token for LLM access.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct McpToken {
    pub id: String,
    pub user_id: String,
    pub token_hash: String,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub expires_at: Option<NaiveDateTime>,
}

/// Permission level for calendar sharing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Permission {
    Read,
    ReadWrite,
}

impl Permission {
    /// Return the wire-format string for this permission level (`"read"` or `"read-write"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Permission::Read => "read",
            Permission::ReadWrite => "read-write",
        }
    }

    /// Parse a permission from its wire-format string. Returns `None` for unknown values.
    pub fn from_str_value(s: &str) -> Option<Self> {
        match s {
            "read" => Some(Permission::Read),
            "read-write" => Some(Permission::ReadWrite),
            _ => None,
        }
    }

    /// Whether this permission allows write operations.
    #[allow(dead_code)]
    pub fn can_write(&self) -> bool {
        matches!(self, Permission::ReadWrite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_as_str() {
        assert_eq!(Permission::Read.as_str(), "read");
        assert_eq!(Permission::ReadWrite.as_str(), "read-write");
    }

    #[test]
    fn test_permission_from_str_value() {
        assert_eq!(Permission::from_str_value("read"), Some(Permission::Read));
        assert_eq!(
            Permission::from_str_value("read-write"),
            Some(Permission::ReadWrite)
        );
        assert_eq!(Permission::from_str_value("write"), None);
        assert_eq!(Permission::from_str_value(""), None);
    }

    #[test]
    fn test_permission_can_write() {
        assert!(!Permission::Read.can_write());
        assert!(Permission::ReadWrite.can_write());
    }

    #[test]
    fn test_permission_equality() {
        assert_eq!(Permission::Read, Permission::Read);
        assert_eq!(Permission::ReadWrite, Permission::ReadWrite);
        assert_ne!(Permission::Read, Permission::ReadWrite);
    }

    #[test]
    fn test_permission_clone_copy() {
        let p = Permission::ReadWrite;
        let q = p;
        assert_eq!(p, q);
    }
}
