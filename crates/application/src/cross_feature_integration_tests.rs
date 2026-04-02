use crate::{
    multi_agent_orchestrator_service::{ApplicationMultiAgentOrchestrator, WorkerRuntime},
    remote_session_service::ApplicationRemoteSessionService,
    spec_contracts::{
        ArchiveResult, CcrClient, CcrClientError, CoordinatorSummary, CreateRemoteSessionRequest,
        MultiAgentOrchestrator, OrchestrationError, RemoteSessionError, RemoteSessionService,
        RemoteSessionState, RemoteSessionSummary, SyncResult, TaskCriticality, TeamMemError,
        TeamMemorySyncService, TriggerError, TriggerOwner, TriggerSchedulerService, TriggerScope,
        TriggerTask, UltraplanError, UltraplanPhase, UltraplanPolicy, UltraplanService,
        WorkerResultNotification, WorkerStatus, WorkerTaskSpec,
    },
    teammem_kairos_service::{
        BackendFailure, TeamMemAuditEvent, TeamMemAuditLog, TeamMemBackend, TeamMemEntry,
        TeamMemKairosService,
    },
    ultraplan_service::{DefaultUltraplanService, UltraplanRuntime},
};
use async_trait::async_trait;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, Notify};

fn remote_summary(session_id: &str, state: RemoteSessionState) -> RemoteSessionSummary {
    RemoteSessionSummary {
        session_id: session_id.to_string(),
        title: Some("cross-feature session".to_string()),
        environment_id: Some("env-test".to_string()),
        state,
    }
}

#[derive(Default)]
struct MockCcrClient {
    get_results: Arc<Mutex<VecDeque<Result<RemoteSessionSummary, CcrClientError>>>>,
}

#[async_trait]
impl CcrClient for MockCcrClient {
    async fn create(
        &self,
        _req: CreateRemoteSessionRequest,
    ) -> Result<RemoteSessionSummary, CcrClientError> {
        Ok(remote_summary(
            "session-created",
            RemoteSessionState::Running,
        ))
    }

    async fn get(&self, _session_id: &str) -> Result<RemoteSessionSummary, CcrClientError> {
        self.get_results
            .lock()
            .await
            .pop_front()
            .expect("missing queued get result")
    }

    async fn archive(&self, _session_id: &str) -> Result<ArchiveResult, CcrClientError> {
        Ok(ArchiveResult::Archived)
    }

    async fn patch_title(&self, _session_id: &str, _title: &str) -> Result<(), CcrClientError> {
        Ok(())
    }
}

#[derive(Default)]
struct UltraplanMockRuntime {
    launch_results: Arc<Mutex<VecDeque<Result<String, UltraplanError>>>>,
    poll_results: Arc<Mutex<VecDeque<Result<UltraplanPhase, UltraplanError>>>>,
    archive_calls: Arc<Mutex<Vec<String>>>,
    stop_calls: Arc<Mutex<Vec<String>>>,
    poll_gate: Option<Arc<Notify>>,
    gate_after_poll_calls: Option<usize>,
    poll_call_count: AtomicUsize,
}

#[async_trait]
impl UltraplanRuntime for UltraplanMockRuntime {
    async fn launch_session(&self, _prompt: &str) -> Result<String, UltraplanError> {
        self.launch_results
            .lock()
            .await
            .pop_front()
            .expect("missing launch result")
    }

    async fn poll_phase(&self, _session_id: &str) -> Result<UltraplanPhase, UltraplanError> {
        let poll_index = self.poll_call_count.fetch_add(1, Ordering::SeqCst);
        if let (Some(gate), Some(gate_after)) = (&self.poll_gate, self.gate_after_poll_calls)
            && poll_index >= gate_after
        {
            gate.notified().await;
        }
        self.poll_results
            .lock()
            .await
            .pop_front()
            .expect("missing poll result")
    }

    async fn stop_session(&self, session_id: &str) -> Result<(), UltraplanError> {
        self.stop_calls.lock().await.push(session_id.to_string());
        Ok(())
    }

    async fn archive_session(&self, session_id: &str) -> Result<(), UltraplanError> {
        self.archive_calls.lock().await.push(session_id.to_string());
        Ok(())
    }
}

#[derive(Default)]
struct RecordingAuditLog {
    events: StdMutex<Vec<TeamMemAuditEvent>>,
}

impl TeamMemAuditLog for RecordingAuditLog {
    fn record(&self, event: TeamMemAuditEvent) {
        self.events.lock().expect("audit lock poisoned").push(event);
    }
}

struct MockTeamMemBackend {
    sync_results: Mutex<VecDeque<Result<SyncResult, BackendFailure>>>,
}

#[async_trait]
impl TeamMemBackend for MockTeamMemBackend {
    async fn pull_once(&self) -> Result<SyncResult, BackendFailure> {
        Ok(SyncResult {
            files_pulled: 0,
            files_pushed: 0,
        })
    }

    async fn push_once(&self) -> Result<SyncResult, BackendFailure> {
        Ok(SyncResult {
            files_pulled: 0,
            files_pushed: 0,
        })
    }

    async fn sync_once(&self) -> Result<SyncResult, BackendFailure> {
        self.sync_results
            .lock()
            .await
            .pop_front()
            .expect("missing sync result")
    }
}

struct InMemoryTriggerScheduler {
    tasks: StdMutex<HashMap<String, TriggerTask>>,
    durable_store: StdMutex<HashMap<String, TriggerTask>>,
    gate_enabled: bool,
}

impl InMemoryTriggerScheduler {
    fn new(gate_enabled: bool) -> Self {
        Self {
            tasks: StdMutex::new(HashMap::new()),
            durable_store: StdMutex::new(HashMap::new()),
            gate_enabled,
        }
    }

    fn restart(&self) -> Self {
        let durable = self
            .durable_store
            .lock()
            .expect("durable store lock poisoned")
            .clone();
        Self {
            tasks: StdMutex::new(durable.clone()),
            durable_store: StdMutex::new(durable),
            gate_enabled: self.gate_enabled,
        }
    }
}

#[async_trait]
impl TriggerSchedulerService for InMemoryTriggerScheduler {
    async fn create(&self, task: TriggerTask) -> Result<TriggerTask, TriggerError> {
        if !self.gate_enabled {
            return Err(TriggerError::GateDisabled);
        }
        if task.scope == TriggerScope::Durable {
            if matches!(task.owner, TriggerOwner::Teammate(_)) {
                return Err(TriggerError::DurableNotAllowedForTeammate);
            }
            self.durable_store
                .lock()
                .expect("durable store lock poisoned")
                .insert(task.id.clone(), task.clone());
        }
        self.tasks
            .lock()
            .expect("task store lock poisoned")
            .insert(task.id.clone(), task.clone());
        Ok(task)
    }

    async fn list(&self) -> Result<Vec<TriggerTask>, TriggerError> {
        if !self.gate_enabled {
            return Err(TriggerError::GateDisabled);
        }
        let mut items: Vec<_> = self
            .tasks
            .lock()
            .expect("task store lock poisoned")
            .values()
            .cloned()
            .collect();
        items.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(items)
    }

    async fn delete(&self, id: &str, actor: TriggerOwner) -> Result<(), TriggerError> {
        if !self.gate_enabled {
            return Err(TriggerError::GateDisabled);
        }
        let tasks = self.tasks.lock().expect("task store lock poisoned");
        let existing = tasks.get(id).ok_or(TriggerError::OwnershipViolation)?;
        if existing.owner != actor {
            return Err(TriggerError::OwnershipViolation);
        }
        drop(tasks);
        self.tasks
            .lock()
            .expect("task store lock poisoned")
            .remove(id);
        self.durable_store
            .lock()
            .expect("durable store lock poisoned")
            .remove(id);
        Ok(())
    }
}

fn trigger_task(id: &str, scope: TriggerScope, owner: TriggerOwner) -> TriggerTask {
    TriggerTask {
        id: id.to_string(),
        cron: "0 9 * * *".to_string(),
        prompt: "do work".to_string(),
        scope,
        owner,
        durable_intent: scope == TriggerScope::Durable,
    }
}

#[derive(Default)]
struct MockWorkerRuntime {
    active_spawns: AtomicUsize,
    max_parallel_spawns: AtomicUsize,
}

#[async_trait]
impl WorkerRuntime for MockWorkerRuntime {
    async fn spawn_worker(&self, task: &WorkerTaskSpec) -> Result<String, String> {
        let active = self.active_spawns.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_parallel_spawns.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        self.active_spawns.fetch_sub(1, Ordering::SeqCst);
        Ok(format!("worker-{}", task.task_id))
    }

    async fn stop_worker(&self, _task_id: &str) -> Result<(), String> {
        Ok(())
    }
}

fn orchestrator_task(task_id: &str, owner_scope: &str) -> WorkerTaskSpec {
    WorkerTaskSpec {
        task_id: task_id.to_string(),
        title: format!("title-{task_id}"),
        prompt: format!("Task {task_id}: edit {owner_scope}. Ownership boundary: {owner_scope}."),
        criticality: TaskCriticality::Sidecar,
        owner_scope: owner_scope.to_string(),
    }
}

fn worker_notification(
    task_id: &str,
    status: WorkerStatus,
    summary: &str,
) -> WorkerResultNotification {
    WorkerResultNotification {
        task_id: task_id.to_string(),
        status,
        summary: summary.to_string(),
    }
}

// BRIDGE x ULTRAPLAN

#[tokio::test]
async fn bridge_ultraplan_launch_failure_after_session_creation_issues_archive_attempt() {
    let runtime = UltraplanMockRuntime {
        launch_results: Arc::new(Mutex::new(vec![Ok("orphan-session".to_string())].into())),
        poll_results: Arc::new(Mutex::new(
            vec![Err(UltraplanError::Transport("poll failed".to_string()))].into(),
        )),
        ..UltraplanMockRuntime::default()
    };
    let archive_calls = runtime.archive_calls.clone();
    let service = DefaultUltraplanService::new(runtime, true);

    let result = service
        .launch(
            "run ultraplan",
            UltraplanPolicy {
                single_active_session: true,
            },
        )
        .await;

    assert!(matches!(result, Err(UltraplanError::LaunchFailed)));
    assert_eq!(
        archive_calls.lock().await.as_slice(),
        ["orphan-session"],
        "launch failure after session creation must archive orphan"
    );
}

#[tokio::test]
async fn bridge_ultraplan_archived_session_cannot_reenter_running_state() {
    let client = MockCcrClient {
        get_results: Arc::new(Mutex::new(
            vec![Ok(remote_summary(
                "session-archived",
                RemoteSessionState::Archived,
            ))]
            .into(),
        )),
    };
    let service = ApplicationRemoteSessionService::new(client, true);

    let result = service.reconcile_resume("session-archived").await;

    assert!(matches!(
        result,
        Err(RemoteSessionError::InvalidStateTransition)
    ));
}

#[tokio::test]
async fn bridge_ultraplan_concurrent_operations_preserve_active_session_pointer_state() {
    let gate = Arc::new(Notify::new());
    let runtime = UltraplanMockRuntime {
        launch_results: Arc::new(Mutex::new(vec![Ok("race-session".to_string())].into())),
        poll_results: Arc::new(Mutex::new(
            vec![Ok(UltraplanPhase::Polling), Ok(UltraplanPhase::Polling)].into(),
        )),
        poll_gate: Some(gate.clone()),
        gate_after_poll_calls: Some(1),
        ..UltraplanMockRuntime::default()
    };

    let service = Arc::new(DefaultUltraplanService::new(runtime, true));
    service
        .launch(
            "start",
            UltraplanPolicy {
                single_active_session: true,
            },
        )
        .await
        .expect("initial launch should succeed");

    let poll_service = Arc::clone(&service);
    let poll_task = tokio::spawn(async move { poll_service.poll("race-session").await });

    tokio::task::yield_now().await;
    service
        .stop("race-session")
        .await
        .expect("stop should clear active pointer");

    gate.notify_one();
    let poll_result = poll_task.await.expect("poll task should complete");
    assert!(
        matches!(poll_result, Err(UltraplanError::ApprovalFailed)),
        "concurrent poll after stop should fail deterministically with ApprovalFailed"
    );

    assert!(
        service.active_marker().await.is_none(),
        "poll result must not resurrect active pointer after stop"
    );
}

// TEAMMEM x KAIROS

#[test]
fn teammem_kairos_secret_detected_content_is_never_emitted_in_prompt_payload() {
    let audit = Arc::new(RecordingAuditLog::default());
    let service = TeamMemKairosService::new(
        MockTeamMemBackend {
            sync_results: Mutex::new(VecDeque::new()),
        },
        2,
        audit,
    );

    let payload = service.build_prompt_payload(&[
        TeamMemEntry {
            path_key: "notes/clean.md".to_string(),
            content: "safe text".to_string(),
        },
        TeamMemEntry {
            path_key: "notes/secret.md".to_string(),
            content: "token sk-12345678901234567890".to_string(),
        },
    ]);

    assert!(payload.contains("notes/clean.md: safe text"));
    assert!(!payload.contains("sk-12345678901234567890"));
}

#[test]
fn teammem_kairos_invalid_path_keys_are_blocked_and_logged_safely() {
    let audit = Arc::new(RecordingAuditLog::default());
    let service = TeamMemKairosService::new(
        MockTeamMemBackend {
            sync_results: Mutex::new(VecDeque::new()),
        },
        1,
        audit.clone(),
    );

    let payload = service.build_prompt_payload(&[
        TeamMemEntry {
            path_key: "../private\nsecret".to_string(),
            content: "must be blocked".to_string(),
        },
        TeamMemEntry {
            path_key: "notes/ok.md".to_string(),
            content: "kept".to_string(),
        },
    ]);

    assert!(payload.contains("notes/ok.md: kept"));
    assert!(!payload.contains("must be blocked"));

    let events = audit.events.lock().expect("audit lock poisoned");
    assert!(events.iter().any(|event| matches!(
        event,
        TeamMemAuditEvent::InvalidPathKeyBlocked { path_key, .. }
            if path_key == "../private?secret"
    )));
}

#[tokio::test]
async fn teammem_kairos_sync_conflict_retries_remain_bounded_and_observable() {
    let audit = Arc::new(RecordingAuditLog::default());
    let service = TeamMemKairosService::new(
        MockTeamMemBackend {
            sync_results: Mutex::new(
                vec![
                    Err(BackendFailure::Conflict),
                    Err(BackendFailure::Conflict),
                    Err(BackendFailure::Conflict),
                ]
                .into(),
            ),
        },
        2,
        audit.clone(),
    );

    let result = service.sync().await;
    assert!(matches!(result, Err(TeamMemError::ConflictExhausted)));

    let observation = service
        .last_sync_observation()
        .await
        .expect("sync observation should be present");
    assert_eq!(observation.attempts, 3);
    assert_eq!(observation.retries, 2);
    assert!(observation.conflict_exhausted);

    let events = audit.events.lock().expect("audit lock poisoned");
    let retry_count = events
        .iter()
        .filter(|event| matches!(event, TeamMemAuditEvent::ConflictRetry { .. }))
        .count();
    assert_eq!(retry_count, 2);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TeamMemAuditEvent::ConflictExhausted { attempts: 3 }))
    );
}

// TRIGGERS x REMOTE

#[tokio::test]
async fn triggers_remote_gate_off_path_keeps_local_tasks_intact() {
    let disabled = InMemoryTriggerScheduler::new(false);
    assert!(matches!(
        disabled
            .create(trigger_task(
                "blocked",
                TriggerScope::SessionOnly,
                TriggerOwner::MainAgent,
            ))
            .await,
        Err(TriggerError::GateDisabled)
    ));

    let enabled = InMemoryTriggerScheduler::new(true);
    enabled
        .create(trigger_task(
            "local-task",
            TriggerScope::SessionOnly,
            TriggerOwner::MainAgent,
        ))
        .await
        .expect("local create should succeed when enabled");

    let tasks = enabled.list().await.expect("listing should work");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, "local-task");
}

#[tokio::test]
async fn triggers_remote_durable_session_only_semantics_survive_restart_semantics() {
    let scheduler = InMemoryTriggerScheduler::new(true);
    scheduler
        .create(trigger_task(
            "session-only",
            TriggerScope::SessionOnly,
            TriggerOwner::MainAgent,
        ))
        .await
        .expect("session task create should succeed");
    scheduler
        .create(trigger_task(
            "durable",
            TriggerScope::Durable,
            TriggerOwner::MainAgent,
        ))
        .await
        .expect("durable task create should succeed");

    let restarted = scheduler.restart();
    let tasks = restarted
        .list()
        .await
        .expect("list after restart should work");

    assert_eq!(tasks.len(), 1, "only durable task should survive restart");
    assert_eq!(tasks[0].id, "durable");
    assert_eq!(tasks[0].scope, TriggerScope::Durable);
}

#[tokio::test]
async fn triggers_remote_ownership_violations_return_deterministic_policy_errors() {
    let scheduler = InMemoryTriggerScheduler::new(true);
    scheduler
        .create(trigger_task(
            "owned",
            TriggerScope::SessionOnly,
            TriggerOwner::Teammate("alice".to_string()),
        ))
        .await
        .expect("task create should succeed");

    let err_one = scheduler
        .delete("owned", TriggerOwner::Teammate("bob".to_string()))
        .await
        .expect_err("delete with wrong owner must fail");
    let err_two = scheduler
        .delete("owned", TriggerOwner::MainAgent)
        .await
        .expect_err("delete with different wrong owner must fail");

    assert!(matches!(err_one, TriggerError::OwnershipViolation));
    assert!(matches!(err_two, TriggerError::OwnershipViolation));
    assert_eq!(err_one.to_string(), "ownership violation");
    assert_eq!(err_one.to_string(), err_two.to_string());
}

// COORDINATOR x MULTIAGENT

#[tokio::test]
async fn coordinator_multiagent_parallel_fanout_occurs_only_when_write_scope_conflicts_absent() {
    let runtime = Arc::new(MockWorkerRuntime::default());
    let orchestrator = ApplicationMultiAgentOrchestrator::new(true, runtime.clone());

    let err = orchestrator
        .spawn_parallel(vec![
            orchestrator_task("conflict-a", "src/shared.rs"),
            orchestrator_task("conflict-b", "src/shared.rs"),
        ])
        .await
        .expect_err("overlapping scopes must be denied");
    assert!(matches!(err, OrchestrationError::PolicyViolation));

    let worker_ids = orchestrator
        .spawn_parallel(vec![
            orchestrator_task("independent-a", "src/a.rs"),
            orchestrator_task("independent-b", "src/b.rs"),
            orchestrator_task("independent-c", "src/c.rs"),
        ])
        .await
        .expect("independent scopes should run in parallel");

    assert_eq!(worker_ids.len(), 3);
    assert!(runtime.max_parallel_spawns.load(Ordering::SeqCst) > 1);
}

#[tokio::test]
async fn coordinator_multiagent_mixed_worker_outcomes_produce_deterministic_aggregate_status() {
    let orchestrator =
        ApplicationMultiAgentOrchestrator::new(true, Arc::new(MockWorkerRuntime::default()));
    orchestrator
        .spawn_parallel(vec![
            orchestrator_task("mix-1", "scope/1"),
            orchestrator_task("mix-2", "scope/2"),
            orchestrator_task("mix-3", "scope/3"),
        ])
        .await
        .expect("spawn should succeed");

    orchestrator
        .handle_notification(worker_notification(
            "mix-1",
            WorkerStatus::Completed,
            "all checks passed",
        ))
        .await
        .expect("notification should succeed");
    orchestrator
        .handle_notification(worker_notification(
            "mix-2",
            WorkerStatus::Failed,
            "compile error",
        ))
        .await
        .expect("notification should succeed");
    orchestrator
        .handle_notification(worker_notification(
            "mix-3",
            WorkerStatus::Cancelled,
            "cancelled by user",
        ))
        .await
        .expect("notification should succeed");

    let task_ids = vec![
        "mix-1".to_string(),
        "mix-2".to_string(),
        "mix-3".to_string(),
    ];
    let first: CoordinatorSummary = orchestrator
        .synthesize_summary(&task_ids)
        .await
        .expect("summary should succeed");
    let second: CoordinatorSummary = orchestrator
        .synthesize_summary(&task_ids)
        .await
        .expect("summary should succeed");

    assert_eq!(first.completed, 1);
    assert_eq!(first.failed, 1);
    assert_eq!(first.completed, second.completed);
    assert_eq!(first.failed, second.failed);
    assert_eq!(first.key_findings, second.key_findings);
    assert_eq!(first.next_actions, second.next_actions);
}

#[tokio::test]
async fn coordinator_multiagent_final_summary_contains_findings_next_steps_and_task_traceability() {
    let orchestrator =
        ApplicationMultiAgentOrchestrator::new(true, Arc::new(MockWorkerRuntime::default()));
    orchestrator
        .spawn_parallel(vec![
            orchestrator_task("trace-1", "scope/trace1"),
            orchestrator_task("trace-2", "scope/trace2"),
            orchestrator_task("trace-3", "scope/trace3"),
        ])
        .await
        .expect("spawn should succeed");

    orchestrator
        .handle_notification(worker_notification(
            "trace-1",
            WorkerStatus::Completed,
            "parser updated",
        ))
        .await
        .expect("notification should succeed");
    orchestrator
        .handle_notification(worker_notification(
            "trace-2",
            WorkerStatus::Failed,
            "failing regression test",
        ))
        .await
        .expect("notification should succeed");
    orchestrator
        .handle_notification(worker_notification(
            "trace-3",
            WorkerStatus::Completed,
            "docs refreshed",
        ))
        .await
        .expect("notification should succeed");

    let summary = orchestrator
        .synthesize_summary(&[
            "trace-1".to_string(),
            "trace-2".to_string(),
            "trace-3".to_string(),
        ])
        .await
        .expect("summary should succeed");

    assert!(
        summary
            .key_findings
            .iter()
            .any(|finding| finding.contains("trace-1")),
        "findings should reference traceable task ids"
    );
    assert!(
        summary
            .key_findings
            .iter()
            .any(|finding| finding.contains("trace-2")),
        "findings should include failed task traceability"
    );
    assert!(
        summary
            .next_actions
            .iter()
            .any(|action| action.contains("trace-2") && action.contains("retry")),
        "next actions should be actionable and reference failed task"
    );
    assert!(
        !summary.next_actions.is_empty(),
        "next actions should always be present"
    );
}
