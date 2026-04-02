use crate::{
    multi_agent_orchestrator_service::{ApplicationMultiAgentOrchestrator, WorkerRuntime},
    spec_contracts::{
        MultiAgentOrchestrator, OrchestrationError, TaskCriticality, WorkerResultNotification,
        WorkerStatus, WorkerTaskSpec,
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

#[derive(Default)]
struct MockWorkerRuntime {
    fail_spawn_for: HashSet<String>,
    active_spawns: AtomicUsize,
    max_parallel_spawns: AtomicUsize,
    stop_calls: Mutex<HashMap<String, usize>>,
}

impl MockWorkerRuntime {
    fn with_spawn_failure(task_id: &str) -> Self {
        Self {
            fail_spawn_for: HashSet::from([task_id.to_string()]),
            ..Self::default()
        }
    }
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

fn sidecar_task(task_id: &str, scope: &str) -> WorkerTaskSpec {
    WorkerTaskSpec {
        task_id: task_id.to_string(),
        title: format!("title-{task_id}"),
        prompt: format!("Task {task_id}: only edit {scope}. Ownership boundary: {scope}."),
        criticality: TaskCriticality::Sidecar,
        owner_scope: scope.to_string(),
    }
}

#[tokio::test]
async fn spawn_parallel_rejects_overlapping_write_scopes() {
    let orchestrator =
        ApplicationMultiAgentOrchestrator::new(true, Arc::new(MockWorkerRuntime::default()));

    let result = orchestrator
        .spawn_parallel(vec![
            sidecar_task("a", "crates/application/src/a.rs"),
            sidecar_task("b", "crates/application/src/a.rs"),
        ])
        .await;

    assert!(matches!(result, Err(OrchestrationError::PolicyViolation)));
}

#[tokio::test]
async fn independent_tasks_are_spawned_in_parallel() {
    let runtime = Arc::new(MockWorkerRuntime::default());
    let orchestrator = ApplicationMultiAgentOrchestrator::new(true, runtime.clone());

    let result = orchestrator
        .spawn_parallel(vec![
            sidecar_task("a", "crates/application/src/a.rs"),
            sidecar_task("b", "crates/application/src/b.rs"),
            sidecar_task("c", "crates/application/src/c.rs"),
        ])
        .await
        .expect("spawn should succeed");

    assert_eq!(result.len(), 3);
    assert!(runtime.max_parallel_spawns.load(Ordering::SeqCst) > 1);
}

#[tokio::test]
async fn synthesize_summary_aggregates_notifications() {
    let orchestrator =
        ApplicationMultiAgentOrchestrator::new(true, Arc::new(MockWorkerRuntime::default()));
    let task_a = sidecar_task("a", "scope/a");
    let task_b = sidecar_task("b", "scope/b");
    let task_c = sidecar_task("c", "scope/c");
    orchestrator
        .spawn_parallel(vec![task_a, task_b, task_c])
        .await
        .expect("spawn should succeed");

    orchestrator
        .handle_notification(WorkerResultNotification {
            task_id: "a".to_string(),
            status: WorkerStatus::Completed,
            summary: "implemented output parsing".to_string(),
        })
        .await
        .expect("notification should be accepted");
    orchestrator
        .handle_notification(WorkerResultNotification {
            task_id: "b".to_string(),
            status: WorkerStatus::Failed,
            summary: "tests failed in CI".to_string(),
        })
        .await
        .expect("notification should be accepted");

    let summary = orchestrator
        .synthesize_summary(&["a".to_string(), "b".to_string(), "c".to_string()])
        .await
        .expect("synthesis should succeed");

    assert_eq!(summary.completed, 1);
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.key_findings.len(), 2);
    assert!(
        summary
            .next_actions
            .iter()
            .any(|line| line.contains("b") && line.contains("retry"))
    );
}

#[tokio::test]
async fn stop_is_idempotent_for_terminal_state() {
    let runtime = Arc::new(MockWorkerRuntime::default());
    let orchestrator = ApplicationMultiAgentOrchestrator::new(true, runtime.clone());
    orchestrator
        .spawn_parallel(vec![sidecar_task("a", "scope/a")])
        .await
        .expect("spawn should succeed");
    orchestrator
        .handle_notification(WorkerResultNotification {
            task_id: "a".to_string(),
            status: WorkerStatus::Completed,
            summary: "done".to_string(),
        })
        .await
        .expect("notification should be accepted");

    orchestrator.stop_task("a").await.expect("idempotent stop");
    orchestrator.stop_task("a").await.expect("idempotent stop");

    let calls = runtime.stop_calls.lock().await;
    assert_eq!(calls.get("a"), None);
}

#[tokio::test]
async fn error_variants_are_reachable() {
    let disabled =
        ApplicationMultiAgentOrchestrator::new(false, Arc::new(MockWorkerRuntime::default()));
    let err = disabled
        .spawn_parallel(vec![sidecar_task("a", "scope/a")])
        .await
        .expect_err("mode should reject orchestration");
    assert!(matches!(err, OrchestrationError::DisabledByMode));

    let orchestrator =
        ApplicationMultiAgentOrchestrator::new(true, Arc::new(MockWorkerRuntime::default()));

    let invalid = WorkerTaskSpec {
        task_id: "bad".to_string(),
        title: "bad".to_string(),
        prompt: "missing boundary marker".to_string(),
        criticality: TaskCriticality::Sidecar,
        owner_scope: "scope/bad".to_string(),
    };
    let err = orchestrator
        .spawn_parallel(vec![invalid])
        .await
        .expect_err("invalid prompt must fail");
    assert!(matches!(err, OrchestrationError::InvalidTaskSpec));

    let err = orchestrator
        .spawn_parallel(vec![WorkerTaskSpec {
            criticality: TaskCriticality::Blocking,
            ..sidecar_task("blocking", "scope/blocking")
        }])
        .await
        .expect_err("blocking work should not be delegated");
    assert!(matches!(err, OrchestrationError::PolicyViolation));

    let err = orchestrator
        .handle_notification(WorkerResultNotification {
            task_id: "".to_string(),
            status: WorkerStatus::Completed,
            summary: "".to_string(),
        })
        .await
        .expect_err("malformed notification");
    assert!(matches!(err, OrchestrationError::NotificationMalformed));

    let err = orchestrator
        .stop_task("unknown")
        .await
        .expect_err("missing task should return not found");
    assert!(matches!(err, OrchestrationError::TaskNotFound));

    let spawn_fail = ApplicationMultiAgentOrchestrator::new(
        true,
        Arc::new(MockWorkerRuntime::with_spawn_failure("boom")),
    );
    let err = spawn_fail
        .spawn_parallel(vec![sidecar_task("boom", "scope/boom")])
        .await
        .expect_err("spawn failures must be mapped");
    assert!(matches!(err, OrchestrationError::SpawnFailed(_)));

    let err = orchestrator
        .synthesize_summary(&[])
        .await
        .expect_err("empty synthesis input should fail");
    assert!(matches!(err, OrchestrationError::SynthesisFailed(_)));
}
