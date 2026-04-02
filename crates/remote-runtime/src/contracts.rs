use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct CreateRemoteSessionRequest {
    pub environment_id: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RemoteSessionSummary {
    pub session_id: String,
    pub state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveResult {
    Archived,
    AlreadyArchived,
}

#[derive(Debug, thiserror::Error)]
pub enum CcrClientError {
    #[error("http error: {0}")]
    Http(String),
    #[error("timeout")]
    Timeout,
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("invalid payload")]
    InvalidPayload,
}

#[async_trait]
pub trait CcrClient: Send + Sync {
    async fn create(
        &self,
        req: CreateRemoteSessionRequest,
    ) -> Result<RemoteSessionSummary, CcrClientError>;
    async fn get(&self, session_id: &str) -> Result<RemoteSessionSummary, CcrClientError>;
    async fn archive(&self, session_id: &str) -> Result<ArchiveResult, CcrClientError>;
    async fn patch_title(&self, session_id: &str, title: &str) -> Result<(), CcrClientError>;
}
