use crate::spec_contracts::{
    CoordinatorSummary, MultiAgentOrchestrator, OrchestrationError, TaskCriticality,
    WorkerResultNotification, WorkerStatus, WorkerTaskSpec,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::RwLock;

#[async_trait::async_trait]
pub trait WorkerRuntime: Send + Sync {
    async fn spawn_worker(&self, task: &WorkerTaskSpec) -> Result<String, String>;
    async fn stop_worker(&self, task_id: &str) -> Result<(), String>;
}

#[derive(Default)]
pub struct InMemoryWorkerRuntime {
    seq: AtomicU64,
}

#[async_trait::async_trait]
impl WorkerRuntime for InMemoryWorkerRuntime {
    async fn spawn_worker(&self, task: &WorkerTaskSpec) -> Result<String, String> {
        let next = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(format!("worker-{next}-{}", task.task_id))
    }

    async fn stop_worker(&self, _task_id: &str) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct WorkerTaskState {
    task: WorkerTaskSpec,
    status: WorkerStatus,
    summary: Option<String>,
}

pub struct ApplicationMultiAgentOrchestrator {
    enabled_by_mode: bool,
    runtime: Arc<dyn WorkerRuntime>,
    tasks: RwLock<HashMap<String, WorkerTaskState>>,
}

impl ApplicationMultiAgentOrchestrator {
    pub fn new(enabled_by_mode: bool, runtime: Arc<dyn WorkerRuntime>) -> Self {
        Self {
            enabled_by_mode,
            runtime,
            tasks: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_in_memory_runtime(enabled_by_mode: bool) -> Self {
        Self::new(enabled_by_mode, Arc::new(InMemoryWorkerRuntime::default()))
    }

    fn ensure_mode_enabled(&self) -> Result<(), OrchestrationError> {
        if self.enabled_by_mode {
            Ok(())
        } else {
            Err(OrchestrationError::DisabledByMode)
        }
    }

    fn validate_task_specs(tasks: &[WorkerTaskSpec]) -> Result<(), OrchestrationError> {
        if tasks.is_empty() {
            return Err(OrchestrationError::InvalidTaskSpec);
        }

        for task in tasks {
            if task.task_id.trim().is_empty()
                || task.title.trim().is_empty()
                || task.prompt.trim().is_empty()
                || task.owner_scope.trim().is_empty()
            {
                return Err(OrchestrationError::InvalidTaskSpec);
            }

            if task.criticality == TaskCriticality::Blocking {
                return Err(OrchestrationError::PolicyViolation);
            }

            if !task.prompt.contains(&task.owner_scope) {
                return Err(OrchestrationError::InvalidTaskSpec);
            }
        }

        Ok(())
    }

    fn has_scope_overlap(tasks: &[WorkerTaskSpec]) -> bool {
        let mut scopes = HashSet::new();
        for task in tasks {
            if !scopes.insert(task.owner_scope.trim()) {
                return true;
            }
        }
        false
    }

    fn is_terminal(status: WorkerStatus) -> bool {
        matches!(
            status,
            WorkerStatus::Completed | WorkerStatus::Failed | WorkerStatus::Cancelled
        )
    }
}

#[async_trait::async_trait]
impl MultiAgentOrchestrator for ApplicationMultiAgentOrchestrator {
    async fn spawn_parallel(
        &self,
        tasks: Vec<WorkerTaskSpec>,
    ) -> Result<Vec<String>, OrchestrationError> {
        self.ensure_mode_enabled()?;
        Self::validate_task_specs(&tasks)?;

        if Self::has_scope_overlap(&tasks) {
            return Err(OrchestrationError::PolicyViolation);
        }

        {
            let state = self.tasks.read().await;
            let active_scopes: HashSet<&str> = state
                .values()
                .filter(|task| !Self::is_terminal(task.status))
                .map(|task| task.task.owner_scope.as_str())
                .collect();

            for task in &tasks {
                // Block if the task_id is currently running (not terminal).
                // Allow re-spawning completed/failed/cancelled tasks for session
                // resumption.
                if let Some(existing) = state.get(&task.task_id)
                    && !Self::is_terminal(existing.status)
                {
                    return Err(OrchestrationError::PolicyViolation);
                }
                if active_scopes.contains(task.owner_scope.as_str()) {
                    return Err(OrchestrationError::PolicyViolation);
                }
            }
        }

        let runtime = self.runtime.clone();
        let spawned = futures::future::join_all(tasks.into_iter().map(|task| {
            let runtime = runtime.clone();
            async move {
                runtime
                    .spawn_worker(&task)
                    .await
                    .map(|worker_id| (task, worker_id))
            }
        }))
        .await;

        let mut created = Vec::new();
        for result in spawned {
            match result {
                Ok(v) => created.push(v),
                Err(msg) => return Err(OrchestrationError::SpawnFailed(msg)),
            }
        }

        let mut worker_ids = Vec::with_capacity(created.len());
        let mut state = self.tasks.write().await;
        for (task, worker_id) in created {
            worker_ids.push(worker_id.clone());
            state.insert(
                task.task_id.clone(),
                WorkerTaskState {
                    task,
                    status: WorkerStatus::Running,
                    summary: None,
                },
            );
        }

        Ok(worker_ids)
    }

    async fn handle_notification(
        &self,
        notification: WorkerResultNotification,
    ) -> Result<(), OrchestrationError> {
        self.ensure_mode_enabled()?;

        if notification.task_id.trim().is_empty() || notification.summary.trim().is_empty() {
            return Err(OrchestrationError::NotificationMalformed);
        }

        let mut state = self.tasks.write().await;
        let task = state
            .get_mut(&notification.task_id)
            .ok_or(OrchestrationError::TaskNotFound)?;
        task.status = notification.status;
        task.summary = Some(notification.summary);
        Ok(())
    }

    async fn synthesize_summary(
        &self,
        task_ids: &[String],
    ) -> Result<CoordinatorSummary, OrchestrationError> {
        self.ensure_mode_enabled()?;

        if task_ids.is_empty() {
            return Err(OrchestrationError::SynthesisFailed(
                "at least one task id is required".to_string(),
            ));
        }

        let state = self.tasks.read().await;
        let mut completed = 0usize;
        let mut failed = 0usize;
        let mut key_findings = Vec::new();
        let mut next_actions = Vec::new();

        for task_id in task_ids {
            let task = state.get(task_id).ok_or(OrchestrationError::TaskNotFound)?;

            match task.status {
                WorkerStatus::Completed => {
                    completed += 1;
                    if let Some(summary) = &task.summary {
                        key_findings.push(format!("{}: {}", task.task.task_id, summary));
                    }
                }
                WorkerStatus::Failed => {
                    failed += 1;
                    if let Some(summary) = &task.summary {
                        key_findings.push(format!("{} (failed): {}", task.task.task_id, summary));
                    }
                    next_actions.push(format!("retry or debug failed task {}", task.task.task_id));
                }
                WorkerStatus::Cancelled => {
                    if let Some(summary) = &task.summary {
                        key_findings
                            .push(format!("{} (cancelled): {}", task.task.task_id, summary));
                    }
                    next_actions.push(format!(
                        "rescope or relaunch cancelled task {}",
                        task.task.task_id
                    ));
                }
                WorkerStatus::Running => {
                    next_actions.push(format!("wait for running task {}", task.task.task_id));
                }
            }
        }

        if next_actions.is_empty() {
            next_actions.push("no immediate follow-up actions".to_string());
        }

        Ok(CoordinatorSummary {
            completed,
            failed,
            key_findings,
            next_actions,
        })
    }

    async fn stop_task(&self, task_id: &str) -> Result<(), OrchestrationError> {
        self.ensure_mode_enabled()?;

        let should_stop = {
            let state = self.tasks.read().await;
            let task = state.get(task_id).ok_or(OrchestrationError::TaskNotFound)?;
            !Self::is_terminal(task.status)
        };

        if !should_stop {
            return Ok(());
        }

        self.runtime
            .stop_worker(task_id)
            .await
            .map_err(OrchestrationError::SpawnFailed)?;

        let mut state = self.tasks.write().await;
        if let Some(task) = state.get_mut(task_id) {
            task.status = WorkerStatus::Cancelled;
            if task.summary.is_none() {
                task.summary = Some("stopped by coordinator".to_string());
            }
        }

        Ok(())
    }
}
