use crate::PortError;
use async_trait::async_trait;
use ccode_domain::session::{Session, SessionId, SessionSummary};
use std::sync::Arc;

#[async_trait]
pub trait SessionRepository: Send + Sync {
    async fn list(&self, limit: usize) -> Result<Vec<SessionSummary>, PortError>;
    async fn find_by_id(&self, id: &SessionId) -> Result<Option<Session>, PortError>;
    async fn save(&self, session: &Session) -> Result<(), PortError>;
    async fn delete(&self, id: &SessionId) -> Result<(), PortError>;
}

/// Blanket impl so `Arc<dyn SessionRepository>` can be passed as a concrete `R: SessionRepository`.
#[async_trait]
impl<T: SessionRepository + ?Sized> SessionRepository for Arc<T> {
    async fn list(&self, limit: usize) -> Result<Vec<SessionSummary>, PortError> {
        (**self).list(limit).await
    }
    async fn find_by_id(&self, id: &SessionId) -> Result<Option<Session>, PortError> {
        (**self).find_by_id(id).await
    }
    async fn save(&self, session: &Session) -> Result<(), PortError> {
        (**self).save(session).await
    }
    async fn delete(&self, id: &SessionId) -> Result<(), PortError> {
        (**self).delete(id).await
    }
}
