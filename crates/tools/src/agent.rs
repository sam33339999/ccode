use crate::worker_monitor::{publish_worker_event, WorkerMonitorEvent};
use crate::ToolRegistry;
use async_trait::async_trait;
use ccode_application::{
    commands::agent_run::{AgentRunCommand, ContextPolicy},
    multi_agent_orchestrator_service::{ApplicationMultiAgentOrchestrator, WorkerRuntime},
    spec_contracts::{
        MultiAgentOrchestrator, OrchestrationError, TaskCriticality, WorkerResultNotification,
        WorkerStatus, WorkerTaskSpec,
    },
};
use ccode_ports::{
    provider::LlmClient,
    repositories::SessionRepository,
    tool::{ToolContext, ToolPort},
    PortError,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
};
use tokio::{sync::Mutex, task::JoinHandle};

#[derive(Default)]
pub struct ManagedWorkerRuntime {
    seq: AtomicU64,
    handles: Mutex<HashMap<String, JoinHandle<()>>>,
}

impl ManagedWorkerRuntime {
    pub async fn attach_handle(&self, task_id: String, handle: JoinHandle<()>) {
        let mut handles = self.handles.lock().await;
        handles.insert(task_id, handle);
    }
}

#[async_trait]
impl WorkerRuntime for ManagedWorkerRuntime {
    async fn spawn_worker(&self, task: &WorkerTaskSpec) -> Result<String, String> {
        let next = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(format!("worker-{next}-{}", task.task_id))
    }

    async fn stop_worker(&self, task_id: &str) -> Result<(), String> {
        let mut handles = self.handles.lock().await;
        if let Some(handle) = handles.remove(task_id) {
            handle.abort();
        }
        Ok(())
    }
}

pub fn coordinator_mode_enabled() -> bool {
    std::env::var(ccode_application::spec_contracts::CLAUDE_CODE_COORDINATOR_MODE)
        .ok()
        .as_deref()
        .and_then(ccode_application::spec_contracts::CoordinatorMode::parse)
        .is_some_and(|mode| mode == ccode_application::spec_contracts::CoordinatorMode::Coordinator)
}

pub fn map_orchestration_error(err: OrchestrationError) -> PortError {
    match err {
        OrchestrationError::DisabledByMode => PortError::Tool(
            "DisabledByMode: orchestration calls require coordinator mode".to_string(),
        ),
        other => PortError::Tool(other.to_string()),
    }
}

pub struct AgentTool {
    provider: Arc<dyn LlmClient>,
    session_repo: Arc<dyn SessionRepository>,
    registry_cell: Arc<OnceLock<Arc<ToolRegistry>>>,
    context_policy: ContextPolicy,
    orchestrator: Arc<dyn MultiAgentOrchestrator>,
    runtime: Arc<ManagedWorkerRuntime>,
}

impl AgentTool {
    pub fn new(
        provider: Arc<dyn LlmClient>,
        session_repo: Arc<dyn SessionRepository>,
        context_policy: ContextPolicy,
        orchestrator: Arc<dyn MultiAgentOrchestrator>,
        runtime: Arc<ManagedWorkerRuntime>,
    ) -> (Self, Arc<OnceLock<Arc<ToolRegistry>>>) {
        let cell = Arc::new(OnceLock::new());
        let tool = Self {
            provider,
            session_repo,
            registry_cell: Arc::clone(&cell),
            context_policy,
            orchestrator,
            runtime,
        };
        (tool, cell)
    }
}

#[async_trait]
impl ToolPort for AgentTool {
    fn name(&self) -> &str {
        "agent"
    }

    fn description(&self) -> &str {
        "Spawn a worker agent from a WorkerTaskSpec in coordinator mode."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string"},
                "title": {"type": "string"},
                "prompt": {"type": "string"},
                "owner_scope": {"type": "string"},
                "criticality": {
                    "type": "string",
                    "enum": ["sidecar", "blocking"],
                    "default": "sidecar"
                },
                "persona": {"type": "string"}
            },
            "required": ["task_id", "title", "prompt", "owner_scope"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, PortError> {
        let task_id = args["task_id"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing: task_id".to_string()))?
            .to_string();
        let title = args["title"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing: title".to_string()))?
            .to_string();
        let prompt = args["prompt"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing: prompt".to_string()))?
            .to_string();
        let owner_scope = args["owner_scope"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing: owner_scope".to_string()))?
            .to_string();
        let criticality = match args["criticality"].as_str().unwrap_or("sidecar") {
            "sidecar" => TaskCriticality::Sidecar,
            "blocking" => TaskCriticality::Blocking,
            _ => return Err(PortError::Tool("invalid: criticality".to_string())),
        };
        let persona = args["persona"].as_str().map(ToString::to_string);

        let task = WorkerTaskSpec {
            task_id: task_id.clone(),
            title,
            prompt: prompt.clone(),
            criticality,
            owner_scope,
        };

        let worker_ids = self
            .orchestrator
            .spawn_parallel(vec![task])
            .await
            .map_err(map_orchestration_error)?;

        let worker_id = worker_ids
            .into_iter()
            .next()
            .ok_or_else(|| PortError::Tool("spawn failed: missing worker id".to_string()))?;

        publish_worker_event(WorkerMonitorEvent {
            task_id: task_id.clone(),
            status: "Running".to_string(),
            summary: Some("spawned by coordinator".to_string()),
            timestamp: std::time::SystemTime::now(),
        });

        let registry = Arc::clone(self.registry_cell.get().ok_or_else(|| {
            PortError::Tool("agent: tool registry not yet initialized".to_string())
        })?);

        let provider = Arc::clone(&self.provider);
        let session_repo = Arc::clone(&self.session_repo);
        let context_policy = self.context_policy.clone();
        let tool_ctx = Arc::new(ctx.clone());
        let orchestrator = Arc::clone(&self.orchestrator);
        let task_id_for_worker = task_id.clone();

        let handle = tokio::spawn(async move {
            let cmd = AgentRunCommand::new(session_repo, provider).with_context(context_policy);

            let on_delta = |_content: String| {};
            let tool_definitions = registry.definitions();

            let execute_tool = move |name: String,
                                     tool_args: Value|
                  -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<String, String>> + Send>,
            > {
                let registry = Arc::clone(&registry);
                let tool_ctx = Arc::clone(&tool_ctx);
                Box::pin(async move {
                    registry
                        .execute(&name, tool_args, &tool_ctx)
                        .await
                        .map_err(|e| e.to_string())
                })
            };

            let result = cmd
                .run(
                    None,
                    persona,
                    prompt,
                    tool_definitions,
                    &on_delta,
                    &execute_tool,
                )
                .await;

            let notification = match result {
                Ok(session_id) => WorkerResultNotification {
                    task_id: task_id_for_worker,
                    status: WorkerStatus::Completed,
                    summary: format!("worker session {} completed", session_id.0),
                },
                Err(err) => WorkerResultNotification {
                    task_id: task_id_for_worker,
                    status: WorkerStatus::Failed,
                    summary: err.to_string(),
                },
            };

            let status_label = match notification.status {
                WorkerStatus::Running => "Running",
                WorkerStatus::Completed => "Completed",
                WorkerStatus::Failed => "Failed",
                WorkerStatus::Cancelled => "Cancelled",
            };
            publish_worker_event(WorkerMonitorEvent {
                task_id: notification.task_id.clone(),
                status: status_label.to_string(),
                summary: Some(notification.summary.clone()),
                timestamp: std::time::SystemTime::now(),
            });
            let _ = orchestrator.handle_notification(notification).await;
        });

        self.runtime.attach_handle(task_id.clone(), handle).await;

        Ok(json!({
            "task_id": task_id,
            "worker_id": worker_id,
            "status": "Running"
        })
        .to_string())
    }
}

pub fn build_orchestrator(runtime: Arc<ManagedWorkerRuntime>) -> Arc<dyn MultiAgentOrchestrator> {
    Arc::new(ApplicationMultiAgentOrchestrator::new(
        coordinator_mode_enabled(),
        runtime,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ccode_ports::{
        provider::{LlmError, LlmRequest, LlmResponse, LlmStream},
        repositories::SessionRepository,
        tool::Permission,
    };
    use std::path::PathBuf;

    struct DummyProvider;

    #[async_trait]
    impl LlmClient for DummyProvider {
        fn name(&self) -> &str {
            "dummy"
        }

        fn default_model(&self) -> &str {
            "dummy-model"
        }

        async fn health_check(&self) -> Result<(), LlmError> {
            Ok(())
        }

        async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
            panic!("not expected in this test")
        }

        async fn stream(&self, _req: LlmRequest) -> Result<LlmStream, LlmError> {
            panic!("not expected in this test")
        }
    }

    struct DummySessionRepo;

    #[async_trait]
    impl SessionRepository for DummySessionRepo {
        async fn list(
            &self,
            _limit: usize,
        ) -> Result<Vec<ccode_domain::session::SessionSummary>, PortError> {
            Ok(Vec::new())
        }

        async fn find_by_id(
            &self,
            _id: &ccode_domain::session::SessionId,
        ) -> Result<Option<ccode_domain::session::Session>, PortError> {
            Ok(None)
        }

        async fn save(&self, _session: &ccode_domain::session::Session) -> Result<(), PortError> {
            Ok(())
        }

        async fn delete(&self, _id: &ccode_domain::session::SessionId) -> Result<(), PortError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn agent_tool_blocks_non_coordinator_mode_with_explicit_disabled_by_mode_error() {
        let runtime = Arc::new(ManagedWorkerRuntime::default());
        let orchestrator = Arc::new(ApplicationMultiAgentOrchestrator::new(
            false,
            runtime.clone(),
        ));
        let (tool, _cell) = AgentTool::new(
            Arc::new(DummyProvider),
            Arc::new(DummySessionRepo),
            ContextPolicy::default(),
            orchestrator,
            runtime,
        );
        let ctx = ToolContext {
            cwd: PathBuf::from("."),
            permission: Permission::default(),
        };

        let err = tool
            .execute(
                json!({
                    "task_id":"w-1",
                    "title":"worker task",
                    "prompt":"edit scope/a only",
                    "owner_scope":"scope/a",
                    "criticality":"sidecar"
                }),
                &ctx,
            )
            .await
            .expect_err("non-coordinator mode should block orchestration");

        assert!(err.to_string().contains("DisabledByMode"));
    }
}
