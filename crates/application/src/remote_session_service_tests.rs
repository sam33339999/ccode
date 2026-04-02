use crate::{
    remote_session_service::ApplicationRemoteSessionService,
    spec_contracts::{
        ArchivePolicy, ArchiveResult, CcrClient, CcrClientError, CreateRemoteSessionRequest,
        RemoteSessionError, RemoteSessionService, RemoteSessionState, RemoteSessionSummary,
        SessionEvent,
    },
};
use std::{collections::VecDeque, sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[derive(Default)]
struct MockCcrClient {
    get_results: Arc<Mutex<VecDeque<Result<RemoteSessionSummary, CcrClientError>>>>,
    archive_results: Arc<Mutex<VecDeque<Result<ArchiveResult, CcrClientError>>>>,
    create_results: Arc<Mutex<VecDeque<Result<RemoteSessionSummary, CcrClientError>>>>,
    patch_results: Arc<Mutex<VecDeque<Result<(), CcrClientError>>>>,
    archive_delay: Option<Duration>,
}

impl MockCcrClient {
    fn with_get(results: Vec<Result<RemoteSessionSummary, CcrClientError>>) -> Self {
        Self {
            get_results: Arc::new(Mutex::new(results.into())),
            ..Self::default()
        }
    }

    fn with_archive(results: Vec<Result<ArchiveResult, CcrClientError>>) -> Self {
        Self {
            archive_results: Arc::new(Mutex::new(results.into())),
            ..Self::default()
        }
    }

    fn with_archive_delay(delay: Duration) -> Self {
        Self {
            archive_delay: Some(delay),
            archive_results: Arc::new(Mutex::new(vec![Ok(ArchiveResult::Archived)].into())),
            ..Self::default()
        }
    }
}

#[async_trait::async_trait]
impl CcrClient for MockCcrClient {
    async fn create(
        &self,
        _req: CreateRemoteSessionRequest,
    ) -> Result<RemoteSessionSummary, CcrClientError> {
        self.create_results
            .lock()
            .await
            .pop_front()
            .expect("missing create result")
    }

    async fn get(&self, _session_id: &str) -> Result<RemoteSessionSummary, CcrClientError> {
        self.get_results
            .lock()
            .await
            .pop_front()
            .expect("missing get result")
    }

    async fn archive(&self, _session_id: &str) -> Result<ArchiveResult, CcrClientError> {
        if let Some(delay) = self.archive_delay {
            tokio::time::sleep(delay).await;
        }

        self.archive_results
            .lock()
            .await
            .pop_front()
            .expect("missing archive result")
    }

    async fn patch_title(&self, _session_id: &str, _title: &str) -> Result<(), CcrClientError> {
        self.patch_results
            .lock()
            .await
            .pop_front()
            .expect("missing patch result")
    }
}

fn sample_summary(state: RemoteSessionState) -> RemoteSessionSummary {
    RemoteSessionSummary {
        session_id: "session_123".to_string(),
        title: Some("t".to_string()),
        environment_id: Some("env_123".to_string()),
        state,
    }
}

fn sample_request() -> CreateRemoteSessionRequest {
    CreateRemoteSessionRequest {
        environment_id: "env_123".to_string(),
        title: Some("title".to_string()),
        permission_mode: Some("default".to_string()),
        events: vec![SessionEvent {
            event_type: "started".to_string(),
            payload_json: serde_json::json!({"ok": true}),
        }],
    }
}

#[tokio::test]
async fn archive_treats_conflict_as_success_equivalent() {
    let client = MockCcrClient::with_archive(vec![Ok(ArchiveResult::AlreadyArchived)]);
    let svc = ApplicationRemoteSessionService::new(client, true);

    let result = svc
        .archive_session(
            "session_123",
            ArchivePolicy {
                timeout: Duration::from_millis(10),
                idempotent: true,
            },
        )
        .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn reconcile_resume_returns_expired_for_expired_session() {
    let client = MockCcrClient::with_get(vec![Ok(sample_summary(RemoteSessionState::Expired))]);
    let svc = ApplicationRemoteSessionService::new(client, true);

    let result = svc.reconcile_resume("session_123").await;

    assert!(matches!(result, Err(RemoteSessionError::Expired)));
}

#[tokio::test]
async fn reconcile_resume_returns_invalid_transition_for_archived() {
    let client = MockCcrClient::with_get(vec![Ok(sample_summary(RemoteSessionState::Archived))]);
    let svc = ApplicationRemoteSessionService::new(client, true);

    let result = svc.reconcile_resume("session_123").await;

    assert!(matches!(
        result,
        Err(RemoteSessionError::InvalidStateTransition)
    ));
}

#[tokio::test]
async fn create_session_maps_not_found_error() {
    let client = MockCcrClient {
        create_results: Arc::new(Mutex::new(
            vec![Err(CcrClientError::Http("status 404: missing".to_string()))].into(),
        )),
        ..MockCcrClient::default()
    };
    let svc = ApplicationRemoteSessionService::new(client, true);

    let result = svc.create_session(sample_request()).await;

    assert!(matches!(result, Err(RemoteSessionError::NotFound)));
}

#[tokio::test]
async fn fetch_maps_auth_unavailable_error() {
    let client = MockCcrClient::with_get(vec![Err(CcrClientError::Unauthorized)]);
    let svc = ApplicationRemoteSessionService::new(client, true);

    let result = svc.fetch_session("session_123").await;

    assert!(matches!(result, Err(RemoteSessionError::AuthUnavailable)));
}

#[tokio::test]
async fn update_title_maps_entitlement_denied_error() {
    let client = MockCcrClient {
        patch_results: Arc::new(Mutex::new(vec![Err(CcrClientError::Forbidden)].into())),
        ..MockCcrClient::default()
    };
    let svc = ApplicationRemoteSessionService::new(client, true);

    let result = svc.update_title("session_123", "new title").await;

    assert!(matches!(result, Err(RemoteSessionError::EntitlementDenied)));
}

#[tokio::test]
async fn archive_maps_upstream_error() {
    let client = MockCcrClient::with_archive(vec![Err(CcrClientError::Http("boom".to_string()))]);
    let svc = ApplicationRemoteSessionService::new(client, true);

    let result = svc
        .archive_session(
            "session_123",
            ArchivePolicy {
                timeout: Duration::from_millis(10),
                idempotent: true,
            },
        )
        .await;

    assert!(matches!(result, Err(RemoteSessionError::Upstream(_))));
}

#[tokio::test]
async fn policy_disabled_short_circuits_as_disabled_by_policy() {
    let client = MockCcrClient::with_get(vec![Ok(sample_summary(RemoteSessionState::Running))]);
    let svc = ApplicationRemoteSessionService::new(client, false);

    let result = svc.fetch_session("session_123").await;

    assert!(matches!(result, Err(RemoteSessionError::DisabledByPolicy)));
}

#[tokio::test]
async fn shutdown_archive_can_degrade_to_best_effort_on_timeout() {
    let client = MockCcrClient::with_archive_delay(Duration::from_millis(50));
    let svc = ApplicationRemoteSessionService::new(client, true);

    let archived = svc
        .archive_on_shutdown_best_effort(
            "session_123",
            ArchivePolicy {
                timeout: Duration::from_millis(1),
                idempotent: true,
            },
        )
        .await;

    assert!(!archived);
}
