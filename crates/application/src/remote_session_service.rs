use crate::spec_contracts::{
    ArchivePolicy, ArchiveResult, CcrClient, CcrClientError, CreateRemoteSessionRequest,
    RemoteSessionError, RemoteSessionService, RemoteSessionState, RemoteSessionSummary,
};

pub struct ApplicationRemoteSessionService<C> {
    client: C,
    policy_enabled: bool,
}

impl<C> ApplicationRemoteSessionService<C>
where
    C: CcrClient,
{
    pub fn new(client: C, policy_enabled: bool) -> Self {
        Self {
            client,
            policy_enabled,
        }
    }

    pub async fn archive_on_shutdown_best_effort(
        &self,
        session_id: &str,
        policy: ArchivePolicy,
    ) -> bool {
        match tokio::time::timeout(policy.timeout, self.archive_session(session_id, policy)).await {
            Ok(Ok(())) => true,
            Ok(Err(_)) | Err(_) => false,
        }
    }

    fn ensure_policy_enabled(&self) -> Result<(), RemoteSessionError> {
        if self.policy_enabled {
            Ok(())
        } else {
            Err(RemoteSessionError::DisabledByPolicy)
        }
    }

    fn map_client_error(err: CcrClientError) -> RemoteSessionError {
        match err {
            CcrClientError::Unauthorized => RemoteSessionError::AuthUnavailable,
            CcrClientError::Forbidden => RemoteSessionError::EntitlementDenied,
            CcrClientError::Http(msg) => {
                if msg.starts_with("status 404:") {
                    RemoteSessionError::NotFound
                } else {
                    RemoteSessionError::Upstream(msg)
                }
            }
            CcrClientError::Timeout => RemoteSessionError::Upstream("timeout".to_string()),
            CcrClientError::InvalidPayload => {
                RemoteSessionError::Upstream("invalid response payload".to_string())
            }
        }
    }
}

#[async_trait::async_trait]
impl<C> RemoteSessionService for ApplicationRemoteSessionService<C>
where
    C: CcrClient + Send + Sync,
{
    async fn create_session(
        &self,
        req: CreateRemoteSessionRequest,
    ) -> Result<RemoteSessionSummary, RemoteSessionError> {
        self.ensure_policy_enabled()?;
        self.client
            .create(req)
            .await
            .map_err(Self::map_client_error)
    }

    async fn fetch_session(
        &self,
        session_id: &str,
    ) -> Result<RemoteSessionSummary, RemoteSessionError> {
        self.ensure_policy_enabled()?;
        self.client
            .get(session_id)
            .await
            .map_err(Self::map_client_error)
    }

    async fn archive_session(
        &self,
        session_id: &str,
        _policy: ArchivePolicy,
    ) -> Result<(), RemoteSessionError> {
        self.ensure_policy_enabled()?;

        match self
            .client
            .archive(session_id)
            .await
            .map_err(Self::map_client_error)?
        {
            ArchiveResult::Archived | ArchiveResult::AlreadyArchived => Ok(()),
        }
    }

    async fn update_title(&self, session_id: &str, title: &str) -> Result<(), RemoteSessionError> {
        self.ensure_policy_enabled()?;
        self.client
            .patch_title(session_id, title)
            .await
            .map_err(Self::map_client_error)
    }

    async fn reconcile_resume(
        &self,
        session_id: &str,
    ) -> Result<RemoteSessionSummary, RemoteSessionError> {
        self.ensure_policy_enabled()?;
        let summary = self
            .client
            .get(session_id)
            .await
            .map_err(Self::map_client_error)?;

        match summary.state {
            RemoteSessionState::Expired => Err(RemoteSessionError::Expired),
            RemoteSessionState::Archived | RemoteSessionState::Failed => {
                Err(RemoteSessionError::InvalidStateTransition)
            }
            _ => Ok(summary),
        }
    }
}
