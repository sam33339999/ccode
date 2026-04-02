use serde::{Deserialize, Serialize};
use crate::message::Message;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub messages: Vec<Message>,
    /// Unix timestamp (ms)
    pub created_at: u64,
    /// Unix timestamp (ms)
    pub updated_at: u64,
    /// Optional pinned provider name for this session
    pub pinned_provider: Option<String>,
}

impl Session {
    pub fn new(id: impl Into<String>, created_at: u64) -> Self {
        Self {
            id: SessionId(id.into()),
            messages: Vec::new(),
            created_at,
            updated_at: created_at,
            pinned_provider: None,
        }
    }

    pub fn add_message(&mut self, message: Message, now: u64) {
        self.messages.push(message);
        self.updated_at = now;
    }

    pub fn message_count(&self) -> usize {
        self.messages.len()
    }
}

/// Lightweight view for list endpoints — avoids loading full message content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub message_count: usize,
    pub created_at: u64,
    pub updated_at: u64,
}

impl From<&Session> for SessionSummary {
    fn from(s: &Session) -> Self {
        Self {
            id: s.id.clone(),
            message_count: s.messages.len(),
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, Role};

    #[test]
    fn new_session_is_empty() {
        let s = Session::new("s1", 1000);
        assert_eq!(s.message_count(), 0);
        assert_eq!(s.id, SessionId("s1".into()));
        assert_eq!(s.created_at, 1000);
        assert_eq!(s.updated_at, 1000);
    }

    #[test]
    fn add_message_increments_count_and_updates_timestamp() {
        let mut s = Session::new("s1", 1000);
        s.add_message(Message::new("m1", Role::User, "hello", 2000), 2000);
        assert_eq!(s.message_count(), 1);
        assert_eq!(s.updated_at, 2000);
    }

    #[test]
    fn summary_reflects_session_state() {
        let mut s = Session::new("s1", 1000);
        s.add_message(Message::new("m1", Role::User, "hi", 2000), 2000);
        s.add_message(Message::new("m2", Role::Assistant, "hello", 3000), 3000);

        let summary = SessionSummary::from(&s);
        assert_eq!(summary.message_count, 2);
        assert_eq!(summary.updated_at, 3000);
        assert_eq!(summary.id, SessionId("s1".into()));
    }
}
