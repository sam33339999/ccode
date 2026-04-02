/// ULTRAPLAN acceptance tests (US-043)
///
/// Covers all scenarios from ultraplan-contract.md §9 Acceptance Matrix:
///   §9.1 Contract tests  – invalid phase transition, concurrent launch AlreadyActive, stop/cleanup determinism
///   §9.2 Integration tests – launch→poll→approved→completed, orphan archive on failure, poll timeout
///   §9.3 CLI behaviour tests – bare command guidance, keyword guard while active, stop clears markers
use crate::{
    spec_contracts::{UltraplanError, UltraplanPhase, UltraplanPolicy, UltraplanService},
    ultraplan_service::{DefaultUltraplanService, UltraplanRuntime},
};
use async_trait::async_trait;
use std::{collections::VecDeque, sync::Arc};
use tokio::sync::Mutex;

// ── helpers ───────────────────────────────────────────────────────────────

fn policy() -> UltraplanPolicy {
    UltraplanPolicy {
        single_active_session: true,
    }
}

#[derive(Default)]
struct MockRuntime {
    launch_results: Arc<Mutex<VecDeque<Result<String, UltraplanError>>>>,
    poll_results: Arc<Mutex<VecDeque<Result<UltraplanPhase, UltraplanError>>>>,
    stop_results: Arc<Mutex<VecDeque<Result<(), UltraplanError>>>>,
    archive_results: Arc<Mutex<VecDeque<Result<(), UltraplanError>>>>,
    archive_calls: Arc<Mutex<Vec<String>>>,
    stop_calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl UltraplanRuntime for MockRuntime {
    async fn launch_session(&self, _prompt: &str) -> Result<String, UltraplanError> {
        self.launch_results
            .lock()
            .await
            .pop_front()
            .expect("missing launch result")
    }

    async fn poll_phase(&self, _session_id: &str) -> Result<UltraplanPhase, UltraplanError> {
        self.poll_results
            .lock()
            .await
            .pop_front()
            .expect("missing poll result")
    }

    async fn stop_session(&self, session_id: &str) -> Result<(), UltraplanError> {
        self.stop_calls.lock().await.push(session_id.to_owned());
        self.stop_results.lock().await.pop_front().unwrap_or(Ok(()))
    }

    async fn archive_session(&self, session_id: &str) -> Result<(), UltraplanError> {
        self.archive_calls.lock().await.push(session_id.to_owned());
        self.archive_results
            .lock()
            .await
            .pop_front()
            .unwrap_or(Ok(()))
    }
}

fn make_service(runtime: MockRuntime, enabled: bool) -> DefaultUltraplanService<MockRuntime> {
    DefaultUltraplanService::new(runtime, enabled)
}

fn happy_runtime() -> MockRuntime {
    MockRuntime {
        launch_results: Arc::new(Mutex::new(vec![Ok("ses-1".to_owned())].into())),
        poll_results: Arc::new(Mutex::new(
            vec![
                Ok(UltraplanPhase::Polling),  // after launch
                Ok(UltraplanPhase::Approved), // poll → approved → completed
            ]
            .into(),
        )),
        ..MockRuntime::default()
    }
}

// ── §9.1 Contract tests ─────────────────────────────────────────────────

/// AC: invalid phase transition rejected with error.
///
/// Transitions from terminal states (Completed, Failed) must be rejected.
/// The service must not allow arbitrary phase jumps.
#[test]
fn contract_invalid_phase_transition_rejected() {
    let disallowed = vec![
        (UltraplanPhase::Completed, UltraplanPhase::Polling),
        (UltraplanPhase::Completed, UltraplanPhase::Launching),
        (UltraplanPhase::Failed, UltraplanPhase::Polling),
        (UltraplanPhase::Failed, UltraplanPhase::Launching),
        (UltraplanPhase::Idle, UltraplanPhase::Polling),
        (UltraplanPhase::Idle, UltraplanPhase::Completed),
        (UltraplanPhase::Launching, UltraplanPhase::Completed),
        (UltraplanPhase::Launching, UltraplanPhase::AwaitingInput),
        (UltraplanPhase::Approved, UltraplanPhase::Polling),
        (UltraplanPhase::Stopping, UltraplanPhase::Polling),
    ];

    for (from, to) in disallowed {
        assert!(
            !DefaultUltraplanService::<MockRuntime>::transition_allowed(from, to),
            "transition {from:?} -> {to:?} should be rejected"
        );
    }
}

/// AC: concurrent launch attempt returns AlreadyActive.
///
/// Two concurrent launches must result in exactly one success and one AlreadyActive.
#[tokio::test]
async fn contract_concurrent_launch_returns_already_active() {
    let runtime = MockRuntime {
        launch_results: Arc::new(Mutex::new(
            vec![Ok("s-1".to_owned()), Ok("s-2".to_owned())].into(),
        )),
        poll_results: Arc::new(Mutex::new(vec![Ok(UltraplanPhase::Polling)].into())),
        ..MockRuntime::default()
    };
    let service = Arc::new(make_service(runtime, true));

    let left = service.launch("prompt", policy());
    let right = service.launch("prompt", policy());
    let (a, b) = tokio::join!(left, right);

    let one_ok = matches!(a, Ok(_)) as u8 + matches!(b, Ok(_)) as u8;
    let one_active = matches!(a, Err(UltraplanError::AlreadyActive)) as u8
        + matches!(b, Err(UltraplanError::AlreadyActive)) as u8;

    assert_eq!(one_ok, 1, "exactly one launch must succeed");
    assert_eq!(one_active, 1, "exactly one must return AlreadyActive");
}

/// AC: stop/cleanup from active state transitions deterministically.
///
/// After stop, active marker must be cleared regardless of runtime stop result.
#[tokio::test]
async fn contract_stop_cleanup_from_active_state_deterministic() {
    // Case 1: stop succeeds
    let runtime_ok = MockRuntime {
        launch_results: Arc::new(Mutex::new(vec![Ok("ses-ok".to_owned())].into())),
        poll_results: Arc::new(Mutex::new(vec![Ok(UltraplanPhase::Polling)].into())),
        stop_results: Arc::new(Mutex::new(vec![Ok(())].into())),
        ..MockRuntime::default()
    };
    let svc_ok = make_service(runtime_ok, true);
    svc_ok.launch("prompt", policy()).await.expect("launch ok");
    assert!(svc_ok.active_marker().await.is_some());
    svc_ok.stop("ses-ok").await.expect("stop ok");
    assert!(
        svc_ok.active_marker().await.is_none(),
        "active marker must be cleared after successful stop"
    );

    // Case 2: stop fails (transport error)
    let runtime_err = MockRuntime {
        launch_results: Arc::new(Mutex::new(vec![Ok("ses-err".to_owned())].into())),
        poll_results: Arc::new(Mutex::new(vec![Ok(UltraplanPhase::Polling)].into())),
        stop_results: Arc::new(Mutex::new(
            vec![Err(UltraplanError::Transport("network".to_owned()))].into(),
        )),
        ..MockRuntime::default()
    };
    let svc_err = make_service(runtime_err, true);
    svc_err.launch("prompt", policy()).await.expect("launch ok");
    assert!(svc_err.active_marker().await.is_some());
    let stop_result = svc_err.stop("ses-err").await;
    assert!(stop_result.is_err(), "stop should return transport error");
    assert!(
        svc_err.active_marker().await.is_none(),
        "active marker must be cleared even when stop fails"
    );
}

// ── §9.2 Integration tests ──────────────────────────────────────────────

/// AC: launch → poll → approved → completed happy path.
///
/// Full lifecycle: launch creates session in Polling, poll returns Approved
/// which auto-resolves to Completed, and active marker is cleared.
#[tokio::test]
async fn integration_launch_poll_approved_completed_happy_path() {
    let service = make_service(happy_runtime(), true);

    let session = service
        .launch("build the feature", policy())
        .await
        .expect("launch succeeds");
    assert_eq!(session.phase, UltraplanPhase::Polling);
    assert_eq!(session.session_id, "ses-1");

    let marker = service.active_marker().await;
    assert!(marker.is_some(), "active marker set after launch");

    let phase = service.poll("ses-1").await.expect("poll succeeds");
    assert_eq!(
        phase,
        UltraplanPhase::Completed,
        "Approved auto-resolves to Completed"
    );

    assert!(
        service.active_marker().await.is_none(),
        "active marker cleared after terminal state"
    );
}

/// AC: launch success + downstream failure triggers orphan archive attempt.
///
/// When remote session is created but subsequent poll fails, the service
/// must attempt to archive the orphaned session.
#[tokio::test]
async fn integration_launch_downstream_failure_triggers_orphan_archive() {
    let runtime = MockRuntime {
        launch_results: Arc::new(Mutex::new(vec![Ok("orphan-ses".to_owned())].into())),
        poll_results: Arc::new(Mutex::new(
            vec![Err(UltraplanError::Transport("poll broke".into()))].into(),
        )),
        ..MockRuntime::default()
    };
    let archive_calls = runtime.archive_calls.clone();
    let service = make_service(runtime, true);

    let result = service.launch("prompt", policy()).await;
    assert!(
        matches!(result, Err(UltraplanError::LaunchFailed)),
        "launch fails when downstream poll fails"
    );

    let calls = archive_calls.lock().await;
    assert_eq!(
        calls.as_slice(),
        &["orphan-ses"],
        "archive must be called for the orphaned session"
    );
}

/// AC: poll timeout path returns PollTimeout.
///
/// When runtime reports PollTimeout, the service propagates it directly.
#[tokio::test]
async fn integration_poll_timeout_returns_poll_timeout() {
    let runtime = MockRuntime {
        poll_results: Arc::new(Mutex::new(vec![Err(UltraplanError::PollTimeout)].into())),
        ..MockRuntime::default()
    };
    let service = make_service(runtime, true);

    let result = service.poll("any-session").await;
    assert!(
        matches!(result, Err(UltraplanError::PollTimeout)),
        "poll timeout must propagate as PollTimeout"
    );
}

// ── §9.3 CLI behaviour tests ────────────────────────────────────────────

/// AC: bare command usage path returns expected guidance.
///
/// When policy is disabled, launch returns DisabledByPolicy as the
/// guidance signal for the CLI to display help text.
#[tokio::test]
async fn cli_bare_command_returns_expected_guidance() {
    let service = make_service(MockRuntime::default(), false);

    let result = service.launch("", policy()).await;
    assert!(
        matches!(result, Err(UltraplanError::DisabledByPolicy)),
        "disabled policy returns guidance error"
    );

    let poll = service.poll("any").await;
    assert!(
        matches!(poll, Err(UltraplanError::DisabledByPolicy)),
        "poll also returns guidance when disabled"
    );

    let stop = service.stop("any").await;
    assert!(
        matches!(stop, Err(UltraplanError::DisabledByPolicy)),
        "stop also returns guidance when disabled"
    );
}

/// AC: keyword-trigger while active does not re-launch.
///
/// If a session is already in Launching or Polling phase, keyword routing
/// must return AlreadyActive instead of launching a new session.
#[tokio::test]
async fn cli_keyword_trigger_while_active_does_not_relaunch() {
    let runtime = MockRuntime {
        launch_results: Arc::new(Mutex::new(vec![Ok("active-ses".to_owned())].into())),
        poll_results: Arc::new(Mutex::new(vec![Ok(UltraplanPhase::Polling)].into())),
        ..MockRuntime::default()
    };
    let service = make_service(runtime, true);

    service
        .launch("initial", policy())
        .await
        .expect("first launch ok");

    let keyword_result = service
        .route_keyword_launch("ultraplan", "re-attempt", policy())
        .await;
    assert!(
        matches!(keyword_result, Err(UltraplanError::AlreadyActive)),
        "keyword launch while active returns AlreadyActive"
    );

    let slash_result = service
        .route_keyword_launch("/ultraplan", "re-attempt", policy())
        .await;
    assert!(
        matches!(slash_result, Err(UltraplanError::AlreadyActive)),
        "slash-keyword launch while active returns AlreadyActive"
    );
}

/// AC: stop command clears local active markers and session URL state.
///
/// After stop, active_marker must return None and the service must accept
/// a fresh launch without AlreadyActive.
#[tokio::test]
async fn cli_stop_clears_markers_and_session_url_state() {
    let runtime = MockRuntime {
        launch_results: Arc::new(Mutex::new(
            vec![Ok("ses-1".to_owned()), Ok("ses-2".to_owned())].into(),
        )),
        poll_results: Arc::new(Mutex::new(
            vec![
                Ok(UltraplanPhase::Polling), // first launch
                Ok(UltraplanPhase::Polling), // second launch
            ]
            .into(),
        )),
        ..MockRuntime::default()
    };
    let stop_calls = runtime.stop_calls.clone();
    let service = make_service(runtime, true);

    service
        .launch("first", policy())
        .await
        .expect("first launch ok");
    assert!(service.active_marker().await.is_some());

    service.stop("ses-1").await.expect("stop ok");
    assert!(
        service.active_marker().await.is_none(),
        "markers cleared after stop"
    );

    let calls = stop_calls.lock().await;
    assert_eq!(
        calls.as_slice(),
        &["ses-1"],
        "stop called on correct session"
    );
    drop(calls);

    // Verify fresh launch works (no AlreadyActive)
    let second = service.launch("second", policy()).await;
    assert!(
        second.is_ok(),
        "fresh launch succeeds after stop cleared state"
    );
}
