use crate::agent::map_orchestration_error;
use async_trait::async_trait;
use ccode_application::spec_contracts::{
    MultiAgentOrchestrator, WorkerResultNotification, WorkerStatus,
};
use ccode_ports::{
    tool::{ToolContext, ToolPort},
    PortError,
};
use serde_json::{json, Value};
use std::sync::Arc;

pub struct TaskStopTool {
    orchestrator: Arc<dyn MultiAgentOrchestrator>,
}

impl TaskStopTool {
    pub fn new(orchestrator: Arc<dyn MultiAgentOrchestrator>) -> Self {
        Self { orchestrator }
    }
}

#[async_trait]
impl ToolPort for TaskStopTool {
    fn name(&self) -> &str {
        "task_stop"
    }

    fn description(&self) -> &str {
        "Stop a running worker task in coordinator mode."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string"},
                "summary": {"type": "string"}
            },
            "required": ["task_id"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<String, PortError> {
        let task_id = args["task_id"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing: task_id".to_string()))?
            .to_string();
        let summary = args["summary"]
            .as_str()
            .unwrap_or("stopped by coordinator")
            .to_string();

        self.orchestrator
            .stop_task(&task_id)
            .await
            .map_err(map_orchestration_error)?;

        let _ = self
            .orchestrator
            .handle_notification(WorkerResultNotification {
                task_id: task_id.clone(),
                status: WorkerStatus::Cancelled,
                summary,
            })
            .await;

        Ok(json!({
            "task_id": task_id,
            "status": "Cancelled"
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::ManagedWorkerRuntime;
    use ccode_application::multi_agent_orchestrator_service::ApplicationMultiAgentOrchestrator;
    use ccode_ports::tool::Permission;
    use std::path::PathBuf;

    #[tokio::test]
    async fn task_stop_blocks_non_coordinator_mode_with_explicit_disabled_by_mode_error() {
        let orchestrator = Arc::new(ApplicationMultiAgentOrchestrator::new(
            false,
            Arc::new(ManagedWorkerRuntime::default()),
        ));
        let tool = TaskStopTool::new(orchestrator);
        let ctx = ToolContext {
            cwd: PathBuf::from("."),
            permission: Permission::default(),
        };

        let err = tool
            .execute(json!({"task_id":"missing"}), &ctx)
            .await
            .expect_err("non-coordinator mode should block task stop");

        assert!(err.to_string().contains("DisabledByMode"));
    }
}
