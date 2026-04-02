use async_trait::async_trait;
use ccode_domain::session::{Session, SessionId, SessionSummary};
use ccode_ports::{PortError, repositories::SessionRepository};
use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct InMemorySessionRepo {
    sessions: Mutex<HashMap<String, Session>>,
}

impl InMemorySessionRepo {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SessionRepository for InMemorySessionRepo {
    async fn list(&self, limit: usize) -> Result<Vec<SessionSummary>, PortError> {
        let sessions = self.sessions.lock().unwrap();
        let mut summaries: Vec<SessionSummary> =
            sessions.values().map(SessionSummary::from).collect();
        summaries.sort_by_key(|s| Reverse(s.updated_at));
        summaries.truncate(limit);
        Ok(summaries)
    }

    async fn find_by_id(&self, id: &SessionId) -> Result<Option<Session>, PortError> {
        let sessions = self.sessions.lock().unwrap();
        Ok(sessions.get(&id.0).cloned())
    }

    async fn save(&self, session: &Session) -> Result<(), PortError> {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(session.id.0.clone(), session.clone());
        Ok(())
    }

    async fn delete(&self, id: &SessionId) -> Result<(), PortError> {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.remove(&id.0);
        Ok(())
    }
}
