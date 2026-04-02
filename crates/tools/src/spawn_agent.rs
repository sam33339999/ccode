use std::sync::{Arc, OnceLock};
use async_trait::async_trait;
use serde_json::{json, Value};
use ccode_application::commands::agent_run::{AgentRunCommand, ContextPolicy};
use ccode_ports::{
    provider::ProviderPort,
    repositories::SessionRepository,
    PortError,
    tool::{ToolContext, ToolPort},
};
use crate::ToolRegistry;

/// Tool that lets the agent delegate a sub-task to a child agent.
///
/// The sub-agent runs with full tool access (same registry) in its own session.
/// All tool calls made by the sub-agent are auto-approved (no stdin prompt).
/// Returns the sub-agent's final response and its session ID.
///
/// The registry is wired via a `OnceLock` to break the circular dependency:
/// the registry must exist before `SpawnAgentTool` can run, but the tool must
/// be registered before the registry is finalised. Bootstrap sets the cell after
/// `Arc<ToolRegistry>` is created.
pub struct SpawnAgentTool {
    provider: Arc<dyn ProviderPort>,
    session_repo: Arc<dyn SessionRepository>,
    /// Filled in by bootstrap after the registry `Arc` is finalized.
    registry_cell: Arc<OnceLock<Arc<ToolRegistry>>>,
    context_policy: ContextPolicy,
}

impl SpawnAgentTool {
    /// Create the tool and return a shared cell that bootstrap must fill with
    /// the completed `Arc<ToolRegistry>` before the tool is first called.
    pub fn new(
        provider: Arc<dyn ProviderPort>,
        session_repo: Arc<dyn SessionRepository>,
        context_policy: ContextPolicy,
    ) -> (Self, Arc<OnceLock<Arc<ToolRegistry>>>) {
        let cell = Arc::new(OnceLock::new());
        let tool = Self {
            provider,
            session_repo,
            registry_cell: Arc::clone(&cell),
            context_policy,
        };
        (tool, cell)
    }
}

#[async_trait]
impl ToolPort for SpawnAgentTool {
    fn name(&self) -> &str { "spawn_agent" }

    fn description(&self) -> &str {
        "Spawn a sub-agent to handle a delegated sub-task independently. \
         The sub-agent has access to all tools and runs in its own session. \
         Returns the sub-agent's final response text and its session ID.\n\
         Parameters:\n\
         - message: The task/prompt for the sub-agent (required)\n\
         - persona: Optional system prompt that defines the sub-agent's role \
           (e.g. \"You are an expert in data analysis\")"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Task for the sub-agent to perform"
                },
                "persona": {
                    "type": "string",
                    "description": "Optional system prompt / role for the sub-agent"
                }
            },
            "required": ["message"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, PortError> {
        let message = args["message"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing: message".into()))?
            .to_string();
        let persona = args["persona"].as_str().map(|s| s.to_string());

        let registry = self
            .registry_cell
            .get()
            .ok_or_else(|| PortError::Tool("spawn_agent: tool registry not yet initialized".into()))?;

        let cmd = AgentRunCommand::new(
            Arc::clone(&self.session_repo),
            Arc::clone(&self.provider),
        )
        .with_context(self.context_policy.clone());

        let tool_definitions = registry.definitions();
        let registry = Arc::clone(registry);
        let tool_ctx = Arc::new(ctx.clone());

        // Collect the sub-agent's streamed response
        let reply = Arc::new(std::sync::Mutex::new(String::new()));
        let reply_clone = reply.clone();

        let on_delta = move |content: String| {
            reply_clone.lock().unwrap().push_str(&content);
        };

        // Sub-agent auto-approves all tool calls — no interactive stdin
        let execute_tool = move |name: String, tool_args: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>> {
            let registry = registry.clone();
            let tool_ctx = tool_ctx.clone();
            Box::pin(async move {
                registry
                    .execute(&name, tool_args, &tool_ctx)
                    .await
                    .map_err(|e| e.to_string())
            })
        };

        let session_id = cmd
            .run(
                None,
                persona,
                message,
                tool_definitions,
                &on_delta,
                &execute_tool,
            )
            .await
            .map_err(|e| PortError::Tool(e.to_string()))?;

        let response = reply.lock().unwrap().clone();
        Ok(json!({
            "session_id": session_id.0,
            "response": response,
        })
        .to_string())
    }
}
