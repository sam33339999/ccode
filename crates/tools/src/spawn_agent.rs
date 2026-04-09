use crate::worker_monitor::{publish_worker_event, WorkerMonitorEvent};
use crate::ToolRegistry;
use async_trait::async_trait;
use ccode_application::commands::agent_run::{AgentRunCommand, ContextPolicy};
use ccode_ports::{
    provider::LlmClient,
    repositories::SessionRepository,
    tool::{ToolContext, ToolPort},
    PortError,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, OnceLock,
};

/// Result of a completed (or failed) sub-agent task.
#[derive(Clone, Debug)]
pub struct AgentTaskResult {
    pub status: String,
    pub session_id: Option<String>,
    pub response: Option<String>,
    pub error: Option<String>,
}

/// Shared store for background sub-agent task results.
///
/// Keyed by task_id. Entries appear once the background task finishes.
pub type AgentTaskStore = Arc<Mutex<HashMap<String, AgentTaskResult>>>;

/// Create a new shared task store (call once at bootstrap, share with both tools).
pub fn new_agent_task_store() -> AgentTaskStore {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Append a line to `~/.ccode/logs/spawn_agent.log`.
fn log_to_file(task_id: &str, level: &str, msg: &str) {
    let Ok(home) = std::env::var("HOME") else {
        return;
    };
    let log_dir = std::path::PathBuf::from(&home).join(".ccode").join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("spawn_agent.log");
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    else {
        return;
    };
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let _ = writeln!(f, "[{now}] [{level}] [{task_id}] {msg}");
}

/// Tool that lets the agent delegate a sub-task to a child agent.
///
/// The sub-agent runs in a background `tokio::spawn` task. `execute()` returns
/// immediately with a `task_id`. Use the companion `get_agent_result` tool to
/// poll for the result.
///
/// The registry is wired via a `OnceLock` to break the circular dependency:
/// the registry must exist before `SpawnAgentTool` can run, but the tool must
/// be registered before the registry is finalised. Bootstrap sets the cell after
/// `Arc<ToolRegistry>` is created.
pub struct SpawnAgentTool {
    provider: Arc<dyn LlmClient>,
    session_repo: Arc<dyn SessionRepository>,
    /// Filled in by bootstrap after the registry `Arc` is finalized.
    registry_cell: Arc<OnceLock<Arc<ToolRegistry>>>,
    context_policy: ContextPolicy,
    task_seq: AtomicU64,
    task_store: AgentTaskStore,
}

impl SpawnAgentTool {
    /// Create the tool and return a shared cell that bootstrap must fill with
    /// the completed `Arc<ToolRegistry>` before the tool is first called.
    pub fn new(
        provider: Arc<dyn LlmClient>,
        session_repo: Arc<dyn SessionRepository>,
        context_policy: ContextPolicy,
        task_store: AgentTaskStore,
    ) -> (Self, Arc<OnceLock<Arc<ToolRegistry>>>) {
        let cell = Arc::new(OnceLock::new());
        let tool = Self {
            provider,
            session_repo,
            registry_cell: Arc::clone(&cell),
            context_policy,
            task_seq: AtomicU64::new(0),
            task_store,
        };
        (tool, cell)
    }
}

#[async_trait]
impl ToolPort for SpawnAgentTool {
    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn description(&self) -> &str {
        "Spawn a sub-agent to handle a delegated sub-task in the background. \
         Returns immediately with a task_id. Use `get_agent_result` to poll for \
         the result.\n\
         Parameters:\n\
         - message: The task/prompt for the sub-agent (required)\n\
         - persona: Optional system prompt that defines the sub-agent's role \
           (e.g. \"You are an expert in data analysis\")\n\
         - session_id: Optional session ID from a previous spawn_agent call. \
           When provided, the sub-agent resumes that session with full conversation \
           history, preserving its persona and context across multiple interactions."
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
                },
                "session_id": {
                    "type": "string",
                    "description": "Session ID from a prior spawn_agent result. \
                        Pass this to continue a conversation with the same sub-agent, \
                        preserving its full history and persona."
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
        let resume_session_id = args["session_id"].as_str().map(|s| s.to_string());

        let registry = self.registry_cell.get().ok_or_else(|| {
            PortError::Tool("spawn_agent: tool registry not yet initialized".into())
        })?;

        let seq = self.task_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let task_id = format!("sub-agent-{seq}");

        // Publish "Running" event so the TUI worker panel can track this sub-agent.
        let summary_hint = if message.len() > 80 {
            format!("{}...", &message[..message.floor_char_boundary(80)])
        } else {
            message.clone()
        };
        publish_worker_event(WorkerMonitorEvent {
            task_id: task_id.clone(),
            status: "Running".to_string(),
            summary: Some(summary_hint.clone()),
            timestamp: std::time::SystemTime::now(),
        });

        log_to_file(&task_id, "INFO", &format!("Spawning: {summary_hint}"));

        // Clone everything needed for the background task
        let provider = Arc::clone(&self.provider);
        let session_repo = Arc::clone(&self.session_repo);
        let context_policy = self.context_policy.clone();
        let registry = Arc::clone(registry);
        let tool_ctx = Arc::new(ctx.clone());
        let task_store = Arc::clone(&self.task_store);
        let bg_task_id = task_id.clone();

        tokio::spawn(async move {
            log_to_file(&bg_task_id, "INFO", "Background task started");

            let cmd = AgentRunCommand::new(session_repo, provider)
                .with_context(context_policy);

            let tool_definitions = registry.definitions();
            log_to_file(
                &bg_task_id,
                "INFO",
                &format!("Tool definitions count: {}", tool_definitions.len()),
            );

            let reply = Arc::new(std::sync::Mutex::new(String::new()));
            let reply_clone = reply.clone();

            let on_delta = move |content: String| {
                reply_clone.lock().unwrap().push_str(&content);
            };

            let reg = registry.clone();
            let ctx2 = tool_ctx.clone();
            let bg_id_for_tool = bg_task_id.clone();
            let execute_tool = move |name: String,
                                     tool_args: Value|
                  -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<String, String>> + Send>,
            > {
                let registry = reg.clone();
                let tool_ctx = ctx2.clone();
                let tid = bg_id_for_tool.clone();
                Box::pin(async move {
                    log_to_file(&tid, "INFO", &format!("Executing tool: {name}"));
                    let r = registry
                        .execute(&name, tool_args, &tool_ctx)
                        .await
                        .map_err(|e| e.to_string());
                    if let Err(ref e) = r {
                        log_to_file(&tid, "ERROR", &format!("Tool {name} failed: {e}"));
                    }
                    r
                })
            };

            log_to_file(&bg_task_id, "INFO", "Calling AgentRunCommand::run ...");

            let result = cmd
                .run(
                    resume_session_id,
                    persona,
                    message,
                    Vec::new(),
                    tool_definitions,
                    &on_delta,
                    &execute_tool,
                )
                .await;

            match result {
                Ok(session_id) => {
                    let response = reply.lock().unwrap().clone();
                    let resp_len = response.len();
                    log_to_file(
                        &bg_task_id,
                        "INFO",
                        &format!("Completed: session={}, response_len={resp_len}", session_id.0),
                    );
                    publish_worker_event(WorkerMonitorEvent {
                        task_id: bg_task_id.clone(),
                        status: "Completed".to_string(),
                        summary: Some(format!("session {}", session_id.0)),
                        timestamp: std::time::SystemTime::now(),
                    });
                    task_store.lock().unwrap().insert(
                        bg_task_id,
                        AgentTaskResult {
                            status: "Completed".to_string(),
                            session_id: Some(session_id.0),
                            response: Some(response),
                            error: None,
                        },
                    );
                }
                Err(e) => {
                    let err_msg = format!("{e:#}");
                    log_to_file(&bg_task_id, "ERROR", &format!("Failed: {err_msg}"));
                    publish_worker_event(WorkerMonitorEvent {
                        task_id: bg_task_id.clone(),
                        status: "Failed".to_string(),
                        summary: Some(err_msg.clone()),
                        timestamp: std::time::SystemTime::now(),
                    });
                    task_store.lock().unwrap().insert(
                        bg_task_id,
                        AgentTaskResult {
                            status: "Failed".to_string(),
                            session_id: None,
                            response: None,
                            error: Some(err_msg),
                        },
                    );
                }
            }
        });

        // Return immediately — the sub-agent runs in the background
        Ok(json!({
            "task_id": task_id,
            "status": "Running",
            "hint": "Use get_agent_result with this task_id to check progress."
        })
        .to_string())
    }
}
