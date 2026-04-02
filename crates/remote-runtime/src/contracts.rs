use async_trait::async_trait;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct CreateRemoteSessionRequest {
    pub environment_id: String,
    pub title: Option<String>,
    pub permission_mode: Option<String>,
    pub events: Vec<SessionEvent>,
}

#[derive(Debug, Clone)]
pub struct RemoteSessionSummary {
    pub session_id: String,
    pub title: Option<String>,
    pub environment_id: Option<String>,
    pub state: RemoteSessionState,
}

#[derive(Debug, Clone)]
pub struct SessionEvent {
    pub event_type: String,
    pub payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Copy)]
pub struct ArchivePolicy {
    pub timeout: Duration,
    pub idempotent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteSessionState {
    Pending,
    Running,
    Idle,
    RequiresAction,
    Archived,
    Expired,
    Failed,
}

impl RemoteSessionState {
    pub const fn can_transition_to(self, target: Self) -> bool {
        match self {
            Self::Pending => matches!(
                target,
                Self::Running | Self::Idle | Self::RequiresAction | Self::Failed
            ),
            Self::Running => matches!(
                target,
                Self::Idle | Self::RequiresAction | Self::Archived | Self::Failed | Self::Expired
            ),
            Self::Idle => matches!(target, Self::Running | Self::Archived | Self::Expired),
            Self::RequiresAction => {
                matches!(target, Self::Running | Self::Archived | Self::Expired)
            }
            Self::Archived | Self::Expired | Self::Failed => false,
        }
    }

    pub fn transition_to(self, target: Self) -> Result<Self, RemoteSessionError> {
        if self.can_transition_to(target) {
            Ok(target)
        } else {
            Err(RemoteSessionError::InvalidStateTransition)
        }
    }
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
    #[error("invalid payload")]
    InvalidPayload,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RemoteSessionError {
    #[error("remote control is disabled by build or gate")]
    DisabledByPolicy,
    #[error("missing entitlement or unsupported login profile")]
    EntitlementDenied,
    #[error("invalid session state transition")]
    InvalidStateTransition,
    #[error("session not found")]
    NotFound,
    #[error("session expired")]
    Expired,
    #[error("auth unavailable")]
    AuthUnavailable,
    #[error("upstream transport error: {0}")]
    Upstream(String),
}

#[async_trait]
pub trait RemoteSessionService: Send + Sync {
    async fn create_session(
        &self,
        req: CreateRemoteSessionRequest,
    ) -> Result<RemoteSessionSummary, RemoteSessionError>;
    async fn fetch_session(
        &self,
        session_id: &str,
    ) -> Result<RemoteSessionSummary, RemoteSessionError>;
    async fn archive_session(
        &self,
        session_id: &str,
        policy: ArchivePolicy,
    ) -> Result<(), RemoteSessionError>;
    async fn update_title(&self, session_id: &str, title: &str) -> Result<(), RemoteSessionError>;
    async fn reconcile_resume(
        &self,
        session_id: &str,
    ) -> Result<RemoteSessionSummary, RemoteSessionError>;
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
