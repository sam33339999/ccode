use crate::spawn_agent::AgentTaskStore;
use async_trait::async_trait;
use ccode_ports::{
    tool::{ToolContext, ToolPort},
    PortError,
};
use serde_json::{json, Value};

/// Tool that polls for the result of a background sub-agent spawned by
/// `spawn_agent`. Returns the current status and, if completed, the
/// sub-agent's response and session ID.
pub struct GetAgentResultTool {
    task_store: AgentTaskStore,
}

impl GetAgentResultTool {
    pub fn new(task_store: AgentTaskStore) -> Self {
        Self { task_store }
    }
}

#[async_trait]
impl ToolPort for GetAgentResultTool {
    fn name(&self) -> &str {
        "get_agent_result"
    }

    fn description(&self) -> &str {
        "Check the result of a background sub-agent task spawned by `spawn_agent`. \
         Returns the status (Running, Completed, Failed), and if finished, \
         the sub-agent's response text and session_id.\n\
         Parameters:\n\
         - task_id: The task_id returned by spawn_agent (required)"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task_id returned by spawn_agent"
                }
            },
            "required": ["task_id"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<String, PortError> {
        let task_id = args["task_id"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing: task_id".into()))?;

        let store = self.task_store.lock().unwrap();
        match store.get(task_id) {
            Some(result) => {
                let mut obj = json!({
                    "task_id": task_id,
                    "status": result.status,
                });
                if let Some(sid) = &result.session_id {
                    obj["session_id"] = json!(sid);
                }
                if let Some(resp) = &result.response {
                    obj["response"] = json!(resp);
                }
                if let Some(err) = &result.error {
                    obj["error"] = json!(err);
                }
                Ok(obj.to_string())
            }
            None => Ok(json!({
                "task_id": task_id,
                "status": "Running",
                "hint": "The sub-agent is still working. Try again shortly."
            })
            .to_string()),
        }
    }
}
