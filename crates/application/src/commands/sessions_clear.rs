use crate::error::AppError;
use ccode_ports::repositories::SessionRepository;

pub struct SessionsClearCommand<R> {
    repo: R,
}

impl<R: SessionRepository> SessionsClearCommand<R> {
    pub fn new(repo: R) -> Self {
        Self { repo }
    }

    pub async fn execute(&self) -> Result<usize, AppError> {
        let sessions = self.repo.list(usize::MAX).await?;
        let mut deleted = 0;
        for summary in sessions {
            self.repo.delete(&summary.id).await?;
            deleted += 1;
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ccode_domain::session::{Session, SessionId, SessionSummary};
    use ccode_ports::PortError;
    use std::sync::Mutex;

    struct MockRepo {
        sessions: Vec<SessionSummary>,
        deleted_ids: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl SessionRepository for MockRepo {
        async fn list(&self, _limit: usize) -> Result<Vec<SessionSummary>, PortError> {
            Ok(self.sessions.clone())
        }
        async fn find_by_id(&self, _id: &SessionId) -> Result<Option<Session>, PortError> {
            Ok(None)
        }
        async fn save(&self, _session: &Session) -> Result<(), PortError> {
            Ok(())
        }
        async fn delete(&self, id: &SessionId) -> Result<(), PortError> {
            self.deleted_ids.lock().unwrap().push(id.0.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn deletes_every_listed_session() {
        let repo = MockRepo {
            sessions: vec![
                SessionSummary {
                    id: SessionId("s1".into()),
                    message_count: 1,
                    created_at: 0,
                    updated_at: 0,
                },
                SessionSummary {
                    id: SessionId("s2".into()),
                    message_count: 2,
                    created_at: 0,
                    updated_at: 0,
                },
            ],
            deleted_ids: Mutex::new(vec![]),
        };
        let deleted = SessionsClearCommand::new(repo).execute().await.unwrap();
        assert_eq!(deleted, 2);
    }
}
