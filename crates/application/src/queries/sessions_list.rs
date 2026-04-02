use ccode_domain::session::SessionSummary;
use ccode_ports::repositories::SessionRepository;
use crate::error::AppError;

pub struct SessionsListQuery<R> {
    repo: R,
}

impl<R: SessionRepository> SessionsListQuery<R> {
    pub fn new(repo: R) -> Self {
        Self { repo }
    }

    pub async fn execute(&self, limit: usize) -> Result<Vec<SessionSummary>, AppError> {
        Ok(self.repo.list(limit).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ccode_domain::session::{Session, SessionId, SessionSummary};
    use ccode_ports::PortError;

    struct MockRepo(Vec<SessionSummary>);

    #[async_trait]
    impl SessionRepository for MockRepo {
        async fn list(&self, limit: usize) -> Result<Vec<SessionSummary>, PortError> {
            Ok(self.0.iter().take(limit).cloned().collect())
        }
        async fn find_by_id(&self, _id: &SessionId) -> Result<Option<Session>, PortError> {
            Ok(None)
        }
        async fn save(&self, _session: &Session) -> Result<(), PortError> {
            Ok(())
        }
        async fn delete(&self, _id: &SessionId) -> Result<(), PortError> {
            Ok(())
        }
    }

    fn make_summary(id: &str) -> SessionSummary {
        SessionSummary {
            id: SessionId(id.into()),
            message_count: 1,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[tokio::test]
    async fn returns_all_sessions_within_limit() {
        let repo = MockRepo(vec![make_summary("s1"), make_summary("s2")]);
        let result = SessionsListQuery::new(repo).execute(10).await.unwrap();
        assert_eq!(result.len(), 2);
    }

    #[tokio::test]
    async fn respects_limit() {
        let repo = MockRepo(vec![make_summary("s1"), make_summary("s2"), make_summary("s3")]);
        let result = SessionsListQuery::new(repo).execute(2).await.unwrap();
        assert_eq!(result.len(), 2);
    }

    #[tokio::test]
    async fn empty_repo_returns_empty_list() {
        let result = SessionsListQuery::new(MockRepo(vec![])).execute(10).await.unwrap();
        assert!(result.is_empty());
    }
}
