/// Bridge mode acceptance tests (US-037)
///
/// Covers all scenarios from bridge-mode-contract.md §8 Acceptance Criteria:
///   §8.1 Contract tests  – transport and policy layer invariants
///   §8.2 Integration tests – multi-step lifecycle flows
///
/// CLI behaviour tests live in crates/cli/src/cmd/bridge.rs.
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

// ── shared test doubles ────────────────────────────────────────────────────

#[derive(Default)]
struct MockCcrClient {
    create_results: Arc<Mutex<VecDeque<Result<RemoteSessionSummary, CcrClientError>>>>,
    get_results: Arc<Mutex<VecDeque<Result<RemoteSessionSummary, CcrClientError>>>>,
    archive_results: Arc<Mutex<VecDeque<Result<ArchiveResult, CcrClientError>>>>,
    patch_results: Arc<Mutex<VecDeque<Result<(), CcrClientError>>>>,
    archive_delay: Option<Duration>,
}

impl MockCcrClient {
    fn with_create(results: Vec<Result<RemoteSessionSummary, CcrClientError>>) -> Self {
        Self {
            create_results: Arc::new(Mutex::new(results.into())),
            ..Self::default()
        }
    }

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

    fn for_happy_path(session_id: &str, state: RemoteSessionState) -> Self {
        let summary = summary_for(session_id, state);
        Self {
            create_results: Arc::new(Mutex::new(vec![Ok(summary.clone())].into())),
            get_results: Arc::new(Mutex::new(vec![Ok(summary)].into())),
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
            .expect("no create result queued")
    }

    async fn get(&self, _session_id: &str) -> Result<RemoteSessionSummary, CcrClientError> {
        self.get_results
            .lock()
            .await
            .pop_front()
            .expect("no get result queued")
    }

    async fn archive(&self, _session_id: &str) -> Result<ArchiveResult, CcrClientError> {
        if let Some(delay) = self.archive_delay {
            tokio::time::sleep(delay).await;
        }
        self.archive_results
            .lock()
            .await
            .pop_front()
            .expect("no archive result queued")
    }

    async fn patch_title(&self, _session_id: &str, _title: &str) -> Result<(), CcrClientError> {
        self.patch_results
            .lock()
            .await
            .pop_front()
            .expect("no patch result queued")
    }
}

fn summary_for(session_id: &str, state: RemoteSessionState) -> RemoteSessionSummary {
    RemoteSessionSummary {
        session_id: session_id.to_string(),
        title: Some("acceptance-test session".to_string()),
        environment_id: Some("env_test".to_string()),
        state,
    }
}

fn sample_create_request() -> CreateRemoteSessionRequest {
    CreateRemoteSessionRequest {
        environment_id: "env_test".to_string(),
        title: Some("acceptance-test".to_string()),
        permission_mode: Some("default".to_string()),
        events: vec![SessionEvent {
            event_type: "started".to_string(),
            payload_json: serde_json::json!({"ok": true}),
        }],
    }
}

fn short_timeout() -> ArchivePolicy {
    ArchivePolicy {
        timeout: Duration::from_millis(10),
        idempotent: true,
    }
}

// ── §8.1 Contract tests ────────────────────────────────────────────────────

/// AC: archive(409) maps to success-equivalent result.
///
/// HTTP 409 Conflict on an archive call means the session is already
/// archived; the service must treat this as Ok(()) — never an error.
#[tokio::test]
async fn contract_archive_409_is_success_equivalent() {
    let client = MockCcrClient::with_archive(vec![Ok(ArchiveResult::AlreadyArchived)]);
    let svc = ApplicationRemoteSessionService::new(client, true);

    let result = svc.archive_session("session_123", short_timeout()).await;

    assert!(
        result.is_ok(),
        "AlreadyArchived (HTTP 409) must be treated as success; got {result:?}"
    );
}

/// AC: session_ and cse_ IDs round-trip correctly through the compatibility adapter.
///
/// The service layer is ID-agnostic: it stores and returns whatever the
/// transport returned, without performing prefix rewriting itself.
/// Prefix translation is a remote-runtime concern (see ccr_client_tests.rs).
#[tokio::test]
async fn contract_session_id_is_preserved_opaquely_by_service() {
    let session_id = "session_abc123";
    let client = MockCcrClient::with_create(vec![Ok(summary_for(
        session_id,
        RemoteSessionState::Running,
    ))]);
    let svc = ApplicationRemoteSessionService::new(client, true);

    let result = svc.create_session(sample_create_request()).await.unwrap();

    assert_eq!(
        result.session_id, session_id,
        "service must not rewrite session IDs; expected {session_id}, got {}",
        result.session_id
    );
}

/// Companion: verify that a cse_-prefixed ID is also preserved opaquely.
#[tokio::test]
async fn contract_cse_id_is_preserved_opaquely_by_service() {
    let cse_id = "cse_abc123";
    let client =
        MockCcrClient::with_get(vec![Ok(summary_for(cse_id, RemoteSessionState::Running))]);
    let svc = ApplicationRemoteSessionService::new(client, true);

    let result = svc.fetch_session(cse_id).await.unwrap();

    assert_eq!(
        result.session_id, cse_id,
        "service must not rewrite cse_ IDs; expected {cse_id}, got {}",
        result.session_id
    );
}

/// AC: missing token produces AuthUnavailable (never panic).
///
/// Unauthorized (HTTP 401) from the transport maps to AuthUnavailable at
/// the service boundary. The call must return Err — not panic.
#[tokio::test]
async fn contract_missing_token_produces_auth_unavailable() {
    let client = MockCcrClient::with_get(vec![Err(CcrClientError::Unauthorized)]);
    let svc = ApplicationRemoteSessionService::new(client, true);

    let result = svc.fetch_session("session_123").await;

    assert!(
        matches!(result, Err(RemoteSessionError::AuthUnavailable)),
        "Unauthorized must map to AuthUnavailable; got {result:?}"
    );
}

/// AC: missing org produces EntitlementDenied (never panic).
///
/// Forbidden (HTTP 403) — which is returned when the org header is absent or
/// unrecognised — maps to EntitlementDenied at the service boundary.
#[tokio::test]
async fn contract_missing_org_produces_entitlement_denied() {
    let client = MockCcrClient {
        patch_results: Arc::new(Mutex::new(vec![Err(CcrClientError::Forbidden)].into())),
        ..MockCcrClient::default()
    };
    let svc = ApplicationRemoteSessionService::new(client, true);

    let result = svc.update_title("session_123", "title").await;

    assert!(
        matches!(result, Err(RemoteSessionError::EntitlementDenied)),
        "Forbidden must map to EntitlementDenied; got {result:?}"
    );
}

// ── §8.2 Integration tests ─────────────────────────────────────────────────

/// AC: create → fetch → archive happy path passes.
///
/// Executes the full lifecycle sequentially against a mock transport and
/// asserts each step succeeds with the expected session ID.
#[tokio::test]
async fn integration_create_fetch_archive_happy_path() {
    let session_id = "session_happy";
    let client = MockCcrClient::for_happy_path(session_id, RemoteSessionState::Running);
    let svc = ApplicationRemoteSessionService::new(client, true);

    // Step 1: create
    let created = svc
        .create_session(sample_create_request())
        .await
        .expect("create must succeed");
    assert_eq!(created.session_id, session_id);
    assert_eq!(created.state, RemoteSessionState::Running);

    // Step 2: fetch
    let fetched = svc
        .fetch_session(session_id)
        .await
        .expect("fetch must succeed");
    assert_eq!(fetched.session_id, session_id);

    // Step 3: archive
    svc.archive_session(session_id, short_timeout())
        .await
        .expect("archive must succeed");
}

/// AC: resume with stale/expired session returns deterministic error classification.
///
/// Expired sessions are not resumable; the service must return Expired,
/// not a generic upstream error or a panic.
#[tokio::test]
async fn integration_resume_with_expired_session_returns_deterministic_error() {
    let client = MockCcrClient::with_get(vec![Ok(summary_for(
        "session_stale",
        RemoteSessionState::Expired,
    ))]);
    let svc = ApplicationRemoteSessionService::new(client, true);

    let result = svc.reconcile_resume("session_stale").await;

    assert!(
        matches!(result, Err(RemoteSessionError::Expired)),
        "expired session must yield RemoteSessionError::Expired; got {result:?}"
    );
}

/// Companion: archived sessions must yield InvalidStateTransition on resume
/// (Archived is a terminal state — cannot re-enter Running).
#[tokio::test]
async fn integration_resume_with_archived_session_returns_invalid_state_transition() {
    let client = MockCcrClient::with_get(vec![Ok(summary_for(
        "session_archived",
        RemoteSessionState::Archived,
    ))]);
    let svc = ApplicationRemoteSessionService::new(client, true);

    let result = svc.reconcile_resume("session_archived").await;

    assert!(
        matches!(result, Err(RemoteSessionError::InvalidStateTransition)),
        "archived session resume must yield InvalidStateTransition; got {result:?}"
    );
}

/// AC: shutdown path performs best-effort archive under timeout budget.
///
/// When the archive call exceeds the policy timeout the best-effort helper
/// must return false (degraded) rather than blocking or panicking.
#[tokio::test]
async fn integration_shutdown_best_effort_archive_under_timeout_budget() {
    let client = MockCcrClient::with_archive_delay(Duration::from_millis(100));
    let svc = ApplicationRemoteSessionService::new(client, true);

    let archived = svc
        .archive_on_shutdown_best_effort(
            "session_shutdown",
            ArchivePolicy {
                timeout: Duration::from_millis(5),
                idempotent: true,
            },
        )
        .await;

    assert!(
        !archived,
        "best-effort archive must degrade gracefully when transport exceeds timeout"
    );
}

/// Companion: best-effort archive returns true when transport completes in time.
#[tokio::test]
async fn integration_shutdown_best_effort_archive_succeeds_within_budget() {
    let client = MockCcrClient::with_archive(vec![Ok(ArchiveResult::Archived)]);
    let svc = ApplicationRemoteSessionService::new(client, true);

    let archived = svc
        .archive_on_shutdown_best_effort(
            "session_ok",
            ArchivePolicy {
                timeout: Duration::from_millis(200),
                idempotent: true,
            },
        )
        .await;

    assert!(
        archived,
        "best-effort archive must return true when transport completes within budget"
    );
}
