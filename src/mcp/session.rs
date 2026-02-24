use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Manages MCP session IDs and their associated user IDs.
#[derive(Debug, Clone)]
pub struct SessionManager {
    sessions: Arc<Mutex<HashMap<String, String>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a new session for a user. Returns the session ID.
    pub fn create_session(&self, user_id: &str) -> String {
        let session_id = Uuid::new_v4().to_string();
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(session_id.clone(), user_id.to_string());
        session_id
    }

    /// Look up the user ID for a session.
    pub fn get_user_id(&self, session_id: &str) -> Option<String> {
        let sessions = self.sessions.lock().unwrap();
        sessions.get(session_id).cloned()
    }

    /// Remove a session.
    pub fn remove_session(&self, session_id: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.remove(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_get_session() {
        let mgr = SessionManager::new();
        let sid = mgr.create_session("user-123");
        assert_eq!(mgr.get_user_id(&sid), Some("user-123".to_string()));
    }

    #[test]
    fn test_remove_session() {
        let mgr = SessionManager::new();
        let sid = mgr.create_session("user-123");
        mgr.remove_session(&sid);
        assert_eq!(mgr.get_user_id(&sid), None);
    }

    #[test]
    fn test_unknown_session() {
        let mgr = SessionManager::new();
        assert_eq!(mgr.get_user_id("nonexistent"), None);
    }
}
