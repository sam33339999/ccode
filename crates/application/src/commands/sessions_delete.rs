use crate::error::AppError;
use ccode_domain::session::SessionId;
use ccode_ports::repositories::SessionRepository;

pub struct SessionsDeleteCommand<R> {
    repo: R,
}

impl<R: SessionRepository> SessionsDeleteCommand<R> {
    pub fn new(repo: R) -> Self {
        Self { repo }
    }

    pub async fn execute(&self, id: String) -> Result<bool, AppError> {
        let session_id = SessionId(id);
        if self.repo.find_by_id(&session_id).await?.is_none() {
            return Ok(false);
        }
        self.repo.delete(&session_id).await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ccode_domain::session::{Session, SessionSummary};
    use ccode_ports::PortError;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct MockRepo {
        found: bool,
        deleted: AtomicBool,
    }

    #[async_trait]
    impl SessionRepository for MockRepo {
        async fn list(&self, _limit: usize) -> Result<Vec<SessionSummary>, PortError> {
            Ok(vec![])
        }
        async fn find_by_id(&self, _id: &SessionId) -> Result<Option<Session>, PortError> {
            if self.found {
                Ok(Some(Session::new("s-1", 0)))
            } else {
                Ok(None)
            }
        }
        async fn save(&self, _session: &Session) -> Result<(), PortError> {
            Ok(())
        }
        async fn delete(&self, _id: &SessionId) -> Result<(), PortError> {
            self.deleted.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn returns_false_when_missing() {
        let repo = MockRepo {
            found: false,
            deleted: AtomicBool::new(false),
        };
        let deleted = SessionsDeleteCommand::new(repo)
            .execute("missing".to_string())
            .await
            .unwrap();
        assert!(!deleted);
    }

    #[tokio::test]
    async fn deletes_when_found() {
        let repo = MockRepo {
            found: true,
            deleted: AtomicBool::new(false),
        };
        let deleted = SessionsDeleteCommand::new(repo)
            .execute("s-1".to_string())
            .await
            .unwrap();
        assert!(deleted);
    }
}
