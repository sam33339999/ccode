/// Multi-agent orchestration acceptance tests (US-041)
///
/// Covers all scenarios from multi-agent-orchestration-contract.md §8 Acceptance Matrix:
///   §8.1 Contract tests  – policy validation, notification parsing, synthesis requirements
///   §8.2 Integration tests – parallel spawning, mixed statuses, idempotent stop
///   §8.3 CLI behaviour tests – coordinator controls, mode blocking, user-visible summary
use crate::{
    multi_agent_orchestrator_service::{ApplicationMultiAgentOrchestrator, WorkerRuntime},
    spec_contracts::{
        CoordinatorSummary, MultiAgentOrchestrator, OrchestrationError, TaskCriticality,
        WorkerResultNotification, WorkerStatus, WorkerTaskSpec,
    },
};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Mutex;

// ── helpers ───────────────────────────────────────────────────────────────

#[derive(Default)]
struct MockWorkerRuntime {
    fail_spawn_for: HashSet<String>,
    active_spawns: AtomicUsize,
    max_parallel_spawns: AtomicUsize,
    stop_calls: Mutex<HashMap<String, usize>>,
}

#[async_trait::async_trait]
impl WorkerRuntime for MockWorkerRuntime {
    async fn spawn_worker(&self, task: &WorkerTaskSpec) -> Result<String, String> {
        if self.fail_spawn_for.contains(&task.task_id) {
            return Err(format!("spawn failed for {}", task.task_id));
        }

        let active = self.active_spawns.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.max_parallel_spawns.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        self.active_spawns.fetch_sub(1, Ordering::SeqCst);

        Ok(format!("worker-{}", task.task_id))
    }

    async fn stop_worker(&self, task_id: &str) -> Result<(), String> {
        let mut calls = self.stop_calls.lock().await;
        let entry = calls.entry(task_id.to_string()).or_insert(0);
        *entry += 1;
        Ok(())
    }
}

fn coordinator_orchestrator() -> ApplicationMultiAgentOrchestrator {
    ApplicationMultiAgentOrchestrator::new(true, Arc::new(MockWorkerRuntime::default()))
}

fn coordinator_orchestrator_with_runtime(
    runtime: Arc<MockWorkerRuntime>,
) -> ApplicationMultiAgentOrchestrator {
    ApplicationMultiAgentOrchestrator::new(true, runtime)
}

fn disabled_orchestrator() -> ApplicationMultiAgentOrchestrator {
    ApplicationMultiAgentOrchestrator::new(false, Arc::new(MockWorkerRuntime::default()))
}

fn task(task_id: &str, scope: &str) -> WorkerTaskSpec {
    WorkerTaskSpec {
        task_id: task_id.to_string(),
        title: format!("title-{task_id}"),
        prompt: format!("Task {task_id}: only edit {scope}. Ownership boundary: {scope}."),
        criticality: TaskCriticality::Sidecar,
        owner_scope: scope.to_string(),
    }
}

fn notification(task_id: &str, status: WorkerStatus, summary: &str) -> WorkerResultNotification {
    WorkerResultNotification {
        task_id: task_id.to_string(),
        status,
        summary: summary.to_string(),
    }
}

// ── §8.1 Contract tests ─────────────────────────────────────────────────

/// AC: parallel fan-out rejects overlapping write scopes when policy forbids them.
///
/// Two tasks targeting the same owner_scope must be rejected with PolicyViolation.
#[tokio::test]
async fn contract_parallel_fanout_rejects_overlapping_write_scopes() {
    let orchestrator = coordinator_orchestrator();

    let result = orchestrator
        .spawn_parallel(vec![
            task("task-1", "crates/domain/src/model.rs"),
            task("task-2", "crates/domain/src/model.rs"),
        ])
        .await;

    assert!(
        matches!(result, Err(OrchestrationError::PolicyViolation)),
        "overlapping write scopes must be rejected; got {:?}",
        result
    );
}

/// AC: notification parsing enforces structured payload fields (task_id, status, summary).
///
/// A notification with empty task_id or empty summary must be rejected as malformed.
#[tokio::test]
async fn contract_notification_parsing_enforces_structured_payload_fields() {
    let orchestrator = coordinator_orchestrator();
    orchestrator
        .spawn_parallel(vec![task("x", "scope/x")])
        .await
        .expect("spawn should succeed");

    // Empty task_id → malformed
    let err = orchestrator
        .handle_notification(WorkerResultNotification {
            task_id: "".to_string(),
            status: WorkerStatus::Completed,
            summary: "has summary".to_string(),
        })
        .await
        .expect_err("empty task_id must be rejected");
    assert!(
        matches!(err, OrchestrationError::NotificationMalformed),
        "expected NotificationMalformed; got {err:?}"
    );

    // Whitespace-only task_id → malformed
    let err = orchestrator
        .handle_notification(WorkerResultNotification {
            task_id: "   ".to_string(),
            status: WorkerStatus::Completed,
            summary: "has summary".to_string(),
        })
        .await
        .expect_err("whitespace-only task_id must be rejected");
    assert!(matches!(err, OrchestrationError::NotificationMalformed));

    // Empty summary → malformed
    let err = orchestrator
        .handle_notification(WorkerResultNotification {
            task_id: "x".to_string(),
            status: WorkerStatus::Completed,
            summary: "".to_string(),
        })
        .await
        .expect_err("empty summary must be rejected");
    assert!(matches!(err, OrchestrationError::NotificationMalformed));

    // Valid notification succeeds
    orchestrator
        .handle_notification(notification("x", WorkerStatus::Completed, "all good"))
        .await
        .expect("valid notification must succeed");
}

/// AC: synthesis requires completed/failed accounting and deterministic summary structure.
///
/// Given a fixed set of notifications, synthesize_summary must produce identical
/// completed/failed counts and identical key_findings/next_actions across invocations.
#[tokio::test]
async fn contract_synthesis_requires_accounting_and_deterministic_structure() {
    let orchestrator = coordinator_orchestrator();
    orchestrator
        .spawn_parallel(vec![
            task("s1", "scope/s1"),
            task("s2", "scope/s2"),
            task("s3", "scope/s3"),
        ])
        .await
        .expect("spawn should succeed");

    orchestrator
        .handle_notification(notification("s1", WorkerStatus::Completed, "lint passed"))
        .await
        .unwrap();
    orchestrator
        .handle_notification(notification("s2", WorkerStatus::Failed, "build broke"))
        .await
        .unwrap();
    orchestrator
        .handle_notification(notification("s3", WorkerStatus::Completed, "tests green"))
        .await
        .unwrap();

    let ids = vec!["s1".to_string(), "s2".to_string(), "s3".to_string()];

    let first = orchestrator
        .synthesize_summary(&ids)
        .await
        .expect("synthesis must succeed");
    let second = orchestrator
        .synthesize_summary(&ids)
        .await
        .expect("synthesis must succeed");

    // Accounting
    assert_eq!(first.completed, 2, "two tasks completed");
    assert_eq!(first.failed, 1, "one task failed");

    // Determinism
    assert_eq!(first.completed, second.completed);
    assert_eq!(first.failed, second.failed);
    assert_eq!(first.key_findings, second.key_findings);
    assert_eq!(first.next_actions, second.next_actions);

    // Structure: key_findings must reference task ids
    assert!(
        first.key_findings.len() >= 2,
        "key_findings must contain entries for terminal tasks"
    );
    // next_actions must contain retry guidance for failed task
    assert!(
        first
            .next_actions
            .iter()
            .any(|a| a.contains("s2") && a.contains("retry")),
        "next_actions must suggest retry for failed task s2"
    );
}

// ── §8.2 Integration tests ──────────────────────────────────────────────

/// AC: spawn 3 independent tasks in parallel and receive 3 notifications.
///
/// Three tasks with distinct scopes must all spawn successfully and the
/// orchestrator must accept a notification for each.
#[tokio::test]
async fn integration_spawn_3_independent_tasks_and_receive_3_notifications() {
    let runtime = Arc::new(MockWorkerRuntime::default());
    let orchestrator = coordinator_orchestrator_with_runtime(runtime.clone());

    let worker_ids = orchestrator
        .spawn_parallel(vec![
            task("w1", "crates/a/src/lib.rs"),
            task("w2", "crates/b/src/lib.rs"),
            task("w3", "crates/c/src/lib.rs"),
        ])
        .await
        .expect("spawn of 3 independent tasks must succeed");

    assert_eq!(worker_ids.len(), 3, "must return 3 worker ids");

    // Verify parallel execution occurred
    assert!(
        runtime.max_parallel_spawns.load(Ordering::SeqCst) > 1,
        "tasks must have been spawned concurrently"
    );

    // Deliver 3 notifications
    for (id, summary) in [
        ("w1", "refactored module A"),
        ("w2", "added tests for B"),
        ("w3", "updated docs for C"),
    ] {
        orchestrator
            .handle_notification(notification(id, WorkerStatus::Completed, summary))
            .await
            .expect(&format!("notification for {id} must be accepted"));
    }

    let summary = orchestrator
        .synthesize_summary(&["w1".to_string(), "w2".to_string(), "w3".to_string()])
        .await
        .expect("synthesis must succeed");

    assert_eq!(
        summary.completed, 3,
        "all 3 tasks must be counted as completed"
    );
    assert_eq!(summary.failed, 0);
}

/// AC: mixed result statuses (completed/failed/cancelled) aggregated correctly.
///
/// The coordinator summary must reflect exact counts and produce appropriate
/// next_actions for non-completed statuses.
#[tokio::test]
async fn integration_mixed_result_statuses_aggregated_correctly() {
    let orchestrator = coordinator_orchestrator();
    orchestrator
        .spawn_parallel(vec![
            task("m1", "scope/m1"),
            task("m2", "scope/m2"),
            task("m3", "scope/m3"),
        ])
        .await
        .expect("spawn should succeed");

    orchestrator
        .handle_notification(notification(
            "m1",
            WorkerStatus::Completed,
            "all tests pass",
        ))
        .await
        .unwrap();
    orchestrator
        .handle_notification(notification("m2", WorkerStatus::Failed, "compile error"))
        .await
        .unwrap();
    orchestrator
        .handle_notification(notification(
            "m3",
            WorkerStatus::Cancelled,
            "user cancelled",
        ))
        .await
        .unwrap();

    let summary = orchestrator
        .synthesize_summary(&["m1".to_string(), "m2".to_string(), "m3".to_string()])
        .await
        .expect("synthesis should succeed");

    assert_eq!(summary.completed, 1);
    assert_eq!(summary.failed, 1);

    // key_findings should mention all tasks
    assert!(
        summary.key_findings.iter().any(|f| f.contains("m1")),
        "key_findings must include completed task m1"
    );
    assert!(
        summary.key_findings.iter().any(|f| f.contains("m2")),
        "key_findings must include failed task m2"
    );
    assert!(
        summary.key_findings.iter().any(|f| f.contains("m3")),
        "key_findings must include cancelled task m3"
    );

    // next_actions for failed → retry, cancelled → rescope
    assert!(
        summary
            .next_actions
            .iter()
            .any(|a| a.contains("m2") && a.contains("retry")),
        "next_actions must suggest retry for failed task m2"
    );
    assert!(
        summary
            .next_actions
            .iter()
            .any(|a| a.contains("m3") && a.contains("rescope")),
        "next_actions must suggest rescope for cancelled task m3"
    );
}

/// AC: stop_task on already-terminal task is idempotent and non-fatal.
///
/// Calling stop_task multiple times on a completed task must succeed without
/// errors and must not invoke the runtime stop_worker.
#[tokio::test]
async fn integration_stop_task_on_terminal_is_idempotent_and_nonfatal() {
    let runtime = Arc::new(MockWorkerRuntime::default());
    let orchestrator = coordinator_orchestrator_with_runtime(runtime.clone());

    orchestrator
        .spawn_parallel(vec![task("done", "scope/done")])
        .await
        .expect("spawn should succeed");

    // Mark task as completed
    orchestrator
        .handle_notification(notification("done", WorkerStatus::Completed, "finished"))
        .await
        .unwrap();

    // Stop multiple times — each must succeed
    orchestrator
        .stop_task("done")
        .await
        .expect("first stop on terminal task must be idempotent");
    orchestrator
        .stop_task("done")
        .await
        .expect("second stop on terminal task must be idempotent");
    orchestrator
        .stop_task("done")
        .await
        .expect("third stop on terminal task must be idempotent");

    // Runtime stop_worker must NOT have been called for terminal tasks
    let calls = runtime.stop_calls.lock().await;
    assert_eq!(
        calls.get("done"),
        None,
        "stop_worker must not be invoked for already-terminal tasks"
    );
}

// ── §8.3 CLI behaviour tests ────────────────────────────────────────────

/// AC: coordinator mode exposes multi-agent controls and task status updates.
///
/// When orchestration mode is enabled (Coordinator), spawn_parallel, handle_notification,
/// synthesize_summary, and stop_task must all be accessible without DisabledByMode errors.
#[tokio::test]
async fn cli_coordinator_mode_exposes_multi_agent_controls() {
    let orchestrator = coordinator_orchestrator();

    // spawn_parallel available
    let worker_ids = orchestrator
        .spawn_parallel(vec![task("ctl-1", "scope/ctl1")])
        .await
        .expect("coordinator mode must allow spawn_parallel");
    assert!(!worker_ids.is_empty());

    // handle_notification available
    orchestrator
        .handle_notification(notification("ctl-1", WorkerStatus::Completed, "done"))
        .await
        .expect("coordinator mode must allow handle_notification");

    // synthesize_summary available
    let summary = orchestrator
        .synthesize_summary(&["ctl-1".to_string()])
        .await
        .expect("coordinator mode must allow synthesize_summary");
    assert_eq!(summary.completed, 1);

    // stop_task available (idempotent on terminal)
    orchestrator
        .stop_task("ctl-1")
        .await
        .expect("coordinator mode must allow stop_task");
}

/// AC: non-coordinator mode blocks orchestration calls with explicit policy error.
///
/// When orchestration is disabled, every trait method must return DisabledByMode.
#[tokio::test]
async fn cli_non_coordinator_mode_blocks_orchestration_with_policy_error() {
    let orchestrator = disabled_orchestrator();

    // spawn_parallel blocked
    let err = orchestrator
        .spawn_parallel(vec![task("blocked", "scope/blocked")])
        .await
        .expect_err("disabled mode must block spawn_parallel");
    assert!(
        matches!(err, OrchestrationError::DisabledByMode),
        "expected DisabledByMode; got {err:?}"
    );

    // handle_notification blocked
    let err = orchestrator
        .handle_notification(notification("blocked", WorkerStatus::Completed, "nope"))
        .await
        .expect_err("disabled mode must block handle_notification");
    assert!(matches!(err, OrchestrationError::DisabledByMode));

    // synthesize_summary blocked
    let err = orchestrator
        .synthesize_summary(&["blocked".to_string()])
        .await
        .expect_err("disabled mode must block synthesize_summary");
    assert!(matches!(err, OrchestrationError::DisabledByMode));

    // stop_task blocked
    let err = orchestrator
        .stop_task("blocked")
        .await
        .expect_err("disabled mode must block stop_task");
    assert!(matches!(err, OrchestrationError::DisabledByMode));
}

/// AC: final user-visible summary includes launched tasks, findings, and next actions.
///
/// The CoordinatorSummary produced by synthesize_summary must have non-empty
/// key_findings and next_actions, with completed/failed counts that add up.
#[tokio::test]
async fn cli_final_summary_includes_launched_tasks_findings_and_next_actions() {
    let orchestrator = coordinator_orchestrator();

    // Launch 3 tasks to simulate a real coordinator workflow
    orchestrator
        .spawn_parallel(vec![
            task("viz-1", "src/parser.rs"),
            task("viz-2", "src/formatter.rs"),
            task("viz-3", "src/validator.rs"),
        ])
        .await
        .expect("spawn should succeed");

    orchestrator
        .handle_notification(notification(
            "viz-1",
            WorkerStatus::Completed,
            "parser refactored with streaming support",
        ))
        .await
        .unwrap();
    orchestrator
        .handle_notification(notification(
            "viz-2",
            WorkerStatus::Failed,
            "formatter regression in edge case",
        ))
        .await
        .unwrap();
    orchestrator
        .handle_notification(notification(
            "viz-3",
            WorkerStatus::Completed,
            "validator now handles unicode",
        ))
        .await
        .unwrap();

    let summary = orchestrator
        .synthesize_summary(&[
            "viz-1".to_string(),
            "viz-2".to_string(),
            "viz-3".to_string(),
        ])
        .await
        .expect("synthesis must succeed for user-visible summary");

    // Launched task accounting
    assert_eq!(
        summary.completed + summary.failed,
        3,
        "completed + failed must account for all terminal tasks (cancelled counted separately)"
    );
    assert_eq!(summary.completed, 2);
    assert_eq!(summary.failed, 1);

    // Findings present and reference tasks
    assert!(
        !summary.key_findings.is_empty(),
        "user-visible summary must include key_findings"
    );
    assert!(
        summary.key_findings.iter().any(|f| f.contains("viz-1")),
        "key_findings must reference completed task viz-1"
    );

    // Next actions present and actionable
    assert!(
        !summary.next_actions.is_empty(),
        "user-visible summary must include next_actions"
    );
    assert!(
        summary
            .next_actions
            .iter()
            .any(|a| a.contains("viz-2") && a.contains("retry")),
        "next_actions must include actionable guidance for failed task viz-2"
    );
}
