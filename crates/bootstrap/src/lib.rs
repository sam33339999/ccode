use ccode_application::commands::agent_run::ContextPolicy;
use ccode_application::spec_contracts::MultiAgentOrchestrator;
use ccode_mcp_runtime::contracts::{CapabilityLevel, DefaultMcpCapabilityPolicy};
use ccode_ports::{
    cron::CronRepository,
    provider::LlmClient,
    repositories::SessionRepository,
    tool::{FsPolicy, Permission, ShellPolicy, ToolContext},
};
use ccode_tools::{
    ToolRegistry,
    agent::{AgentTool, ManagedWorkerRuntime, build_orchestrator},
    fs::{FsEditTool, FsGlobTool, FsGrepTool, FsListTool, FsReadTool, FsWriteTool},
    mcp::{McpServerLaunch, discover_mcp_tools},
    shell::ShellTool,
    spawn_agent::SpawnAgentTool,
    task_stop::TaskStopTool,
    web::{BrowserTool, WebFetchTool},
};
use std::path::PathBuf;
use std::sync::Arc;

pub mod exports {
    pub use ccode_cron::{next_run_ms, parse_natural_schedule};
    pub use ccode_domain::cron::{CronJob, CronJobId};
    pub use ccode_domain::message::{Message, Role};
    pub use ccode_ports::{
        cron::CronRepository,
        provider::{LlmClient, LlmRequest},
    };
}

/// Shared application state passed into every request handler.
pub struct AppState {
    pub session_repo: Arc<dyn SessionRepository>,
    pub cron_repo: Arc<dyn CronRepository>,
    pub provider: Option<Arc<dyn LlmClient>>,
    pub tool_registry: Arc<ToolRegistry>,
    pub permission: Permission,
    pub cwd: PathBuf,
    pub context_policy: ContextPolicy,
}

impl AppState {
    /// Build a `ToolContext` using this state's cwd and sandbox permission.
    pub fn tool_ctx(&self) -> ToolContext {
        ToolContext {
            cwd: self.cwd.clone(),
            permission: self.permission.clone(),
        }
    }
}

/// Build a ToolRegistry with all standard tools registered.
/// Pass `cron_repo` and `provider` to enable the `cron_create` tool;
/// if either is `None` the tool is omitted.
///
/// `spawn_agent` is NOT registered here — call [`wire_spawn_agent`] after wrapping
/// the registry in `Arc` to complete the two-phase bootstrap.
pub fn build_tool_registry(
    _cwd: PathBuf,
    cron_repo: Option<Arc<dyn ccode_ports::cron::CronRepository>>,
    provider: Option<Arc<dyn ccode_ports::provider::LlmClient>>,
    discovered_mcp_tools: Vec<ccode_tools::mcp::DiscoveredMcpTool>,
) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(FsReadTool));
    registry.register(Arc::new(FsWriteTool));
    registry.register(Arc::new(FsEditTool));
    registry.register(Arc::new(FsListTool));
    registry.register(Arc::new(FsGrepTool));
    registry.register(Arc::new(FsGlobTool));
    registry.register(Arc::new(ShellTool));
    registry.register(Arc::new(WebFetchTool::new()));
    registry.register(Arc::new(BrowserTool::new()));
    if let (Some(repo), Some(prov)) = (cron_repo, provider) {
        registry.register(Arc::new(ccode_tools::cron_create::CronCreateTool::new(
            repo, prov,
        )));
    }
    for tool in discovered_mcp_tools {
        registry.register_with_source(tool.source, tool.adapter);
    }
    registry
}

/// Dev/test wiring — in-memory everything, no config required.
pub fn wire_dev() -> AppState {
    use ccode_cron::FileCronRepo;
    use ccode_session::in_memory::InMemorySessionRepo;
    let cwd = std::env::current_dir().unwrap_or_default();
    let cron_dir = std::env::temp_dir().join("ccode-dev-cron");
    AppState {
        session_repo: Arc::new(InMemorySessionRepo::new()),
        cron_repo: Arc::new(FileCronRepo::new(cron_dir).expect("cron dir")),
        provider: None,
        tool_registry: Arc::new(build_tool_registry(cwd.clone(), None, None, Vec::new())),
        permission: Permission::default(),
        cwd,
        context_policy: ContextPolicy::default(),
    }
}

/// Two-phase helper: registers `spawn_agent` into an already-built registry.
///
/// Must be called after the registry is wrapped in `Arc` so that the tool can
/// hold a back-reference to the same `Arc`.  Returns the same `Arc` (with the
/// tool now visible inside via the `OnceLock`).
fn wire_spawn_agent(
    registry: Arc<ToolRegistry>,
    provider: Arc<dyn LlmClient>,
    session_repo: Arc<dyn SessionRepository>,
    context_policy: ContextPolicy,
    orchestrator: Arc<dyn MultiAgentOrchestrator>,
    worker_runtime: Arc<ManagedWorkerRuntime>,
) -> Arc<ToolRegistry> {
    let provider_for_agent = Arc::clone(&provider);
    let session_repo_for_agent = Arc::clone(&session_repo);
    let context_policy_for_agent = context_policy.clone();

    // SpawnAgentTool::new returns the tool + a shared OnceLock cell
    let (spawn_tool, cell) = SpawnAgentTool::new(provider, session_repo, context_policy);
    let (agent_tool, agent_cell) = AgentTool::new(
        provider_for_agent,
        session_repo_for_agent,
        context_policy_for_agent,
        orchestrator.clone(),
        worker_runtime,
    );

    // We need a mutable ToolRegistry — unwrap the Arc (only one owner at this point)
    // and re-wrap after registration.
    let mut inner = Arc::try_unwrap(registry).unwrap_or_else(|_| {
        panic!("wire_spawn_agent must be called before the registry Arc is cloned")
    });
    inner.register(Arc::new(spawn_tool));
    inner.register(Arc::new(agent_tool));
    inner.register(Arc::new(TaskStopTool::new(orchestrator)));
    let registry = Arc::new(inner);

    // Wire the back-reference — must succeed because the cell is brand-new
    cell.set(Arc::clone(&registry))
        .unwrap_or_else(|_| panic!("registry cell already set"));
    agent_cell
        .set(Arc::clone(&registry))
        .unwrap_or_else(|_| panic!("registry cell already set"));

    registry
}

/// Production wiring for gateway/server — loads config from `~/.ccode/config.toml`.
/// Uses `sandbox.cwd` from config as the working directory (falls back to `current_dir`).
/// For CLI usage, call [`wire_from_config_with_cwd`] with `Some(current_dir())` instead.
pub fn wire_from_config() -> Result<AppState, WireError> {
    wire_from_config_with_cwd(None)
}

/// Production wiring — loads config from `~/.ccode/config.toml`.
///
/// `cwd_override`: if `Some`, always use this as the working directory (CLI usage).
/// If `None`, falls back to `sandbox.cwd` in config, then `std::env::current_dir()`
/// (gateway/server usage where no invocation directory exists).
pub fn wire_from_config_with_cwd(cwd_override: Option<PathBuf>) -> Result<AppState, WireError> {
    use ccode_cron::FileCronRepo;
    use ccode_provider::factory;
    use ccode_session::jsonl::FileSessionRepo;

    let config = ccode_config::load().map_err(WireError::Config)?;
    let base = ccode_config::paths::ccode_dir();

    let session_repo = FileSessionRepo::new(base.join("sessions"))
        .map_err(|e| WireError::Storage(e.to_string()))?;

    let cron_repo =
        FileCronRepo::new(base.join("cron")).map_err(|e| WireError::Storage(e.to_string()))?;

    let provider = match factory::build_default(&config) {
        Ok(p) => Some(p),
        Err(factory::FactoryError::NotConfigured(_)) => {
            tracing::warn!("no LLM provider configured — provider features unavailable");
            None
        }
        Err(e) => return Err(WireError::Provider(e.to_string())),
    };

    let cwd = cwd_override.unwrap_or_else(|| {
        config
            .sandbox
            .as_ref()
            .and_then(|s| s.cwd.as_deref())
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
    });

    let permission = permission_from_sandbox(config.sandbox.as_ref());
    let mcp_servers: Vec<McpServerLaunch> = config
        .mcp
        .servers
        .iter()
        .map(|server| McpServerLaunch {
            name: server.name.clone(),
            command: server.command.clone(),
            args: server.args.clone(),
            declared_capabilities: server
                .declared_capabilities
                .iter()
                .map(|capability| match capability.as_str() {
                    "privileged_computer_use" => CapabilityLevel::PrivilegedComputerUse,
                    _ => CapabilityLevel::Standard,
                })
                .collect(),
            enable_computer_use: server.enable_computer_use,
        })
        .collect();
    let discovered_mcp_tools = if mcp_servers.is_empty() {
        Vec::new()
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| WireError::McpRuntime(e.to_string()))?;
        let policy = DefaultMcpCapabilityPolicy::new(
            config.mcp.enable_chicago_mcp_feature_gate,
            config.mcp.allow_privileged_computer_use,
        );
        match rt.block_on(discover_mcp_tools(
            &mcp_servers,
            &policy,
            config.mcp.enable_chicago_mcp_feature_gate,
        )) {
            Ok(tools) => tools,
            Err(e) => {
                tracing::warn!("MCP discovery failed: {e}");
                Vec::new()
            }
        }
    };

    let cron_repo: Arc<dyn ccode_ports::cron::CronRepository> = Arc::new(cron_repo);
    let session_repo: Arc<dyn SessionRepository> = Arc::new(session_repo);
    let provider_arc = provider;
    let context_policy = context_policy_from_config(&config.context);

    let tool_registry = Arc::new(build_tool_registry(
        cwd.clone(),
        Some(Arc::clone(&cron_repo)),
        provider_arc.clone(),
        discovered_mcp_tools,
    ));
    let worker_runtime = Arc::new(ManagedWorkerRuntime::default());
    let orchestrator = build_orchestrator(worker_runtime.clone());

    // Two-phase: register spawn_agent with back-reference to the completed registry
    let tool_registry = if let Some(prov) = provider_arc.clone() {
        wire_spawn_agent(
            tool_registry,
            prov,
            Arc::clone(&session_repo),
            context_policy.clone(),
            orchestrator,
            worker_runtime,
        )
    } else {
        tool_registry
    };

    Ok(AppState {
        session_repo,
        cron_repo,
        provider: provider_arc,
        tool_registry,
        permission,
        cwd,
        context_policy,
    })
}

fn context_policy_from_config(cfg: &ccode_config::schema::ContextConfig) -> ContextPolicy {
    let defaults = ContextPolicy::default();
    ContextPolicy {
        compress_chars_threshold: cfg
            .compress_chars_threshold
            .unwrap_or(defaults.compress_chars_threshold),
        keep_recent_messages: cfg
            .keep_recent_messages
            .unwrap_or(defaults.keep_recent_messages),
        tool_result_max_chars: cfg
            .tool_result_max_chars
            .unwrap_or(defaults.tool_result_max_chars),
    }
}

fn permission_from_sandbox(sandbox: Option<&ccode_config::schema::SandboxConfig>) -> Permission {
    let Some(s) = sandbox else {
        return Permission::default();
    };
    Permission {
        fs_read: match s.fs_read.as_deref() {
            Some("any") => FsPolicy::Any,
            Some("cwd") => FsPolicy::Cwd,
            Some("none") | None => FsPolicy::None,
            Some(_) => FsPolicy::None,
        },
        fs_write: match s.fs_write.as_deref() {
            Some("any") => FsPolicy::Any,
            Some("cwd") => FsPolicy::Cwd,
            Some("none") | None => FsPolicy::None,
            Some(_) => FsPolicy::None,
        },
        shell: match s.shell.as_deref() {
            Some("any") => ShellPolicy::Any,
            Some("none") | None => ShellPolicy::None,
            Some(list) => {
                let commands: Vec<String> = list
                    .split(',')
                    .map(|command| command.trim())
                    .filter(|command| !command.is_empty())
                    .map(ToString::to_string)
                    .collect();
                if commands.is_empty() {
                    ShellPolicy::None
                } else {
                    ShellPolicy::Allowlist(commands)
                }
            }
        },
        web_fetch: s.web_fetch.unwrap_or(false),
        browser: s.browser.unwrap_or(false),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("config error: {0}")]
    Config(#[from] ccode_config::ConfigError),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("mcp runtime error: {0}")]
    McpRuntime(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use ccode_config::schema::SandboxConfig;

    #[test]
    fn sandbox_defaults_to_fail_closed_when_missing() {
        let permission = permission_from_sandbox(None);

        assert!(matches!(permission.fs_read, FsPolicy::None));
        assert!(matches!(permission.fs_write, FsPolicy::None));
        assert!(matches!(permission.shell, ShellPolicy::None));
        assert!(!permission.web_fetch);
        assert!(!permission.browser);
    }

    #[test]
    fn sandbox_invalid_values_fail_closed() {
        let sandbox = SandboxConfig {
            cwd: None,
            fs_read: Some("unexpected".to_string()),
            fs_write: Some("unexpected".to_string()),
            shell: Some("".to_string()),
            web_fetch: None,
            browser: None,
        };

        let permission = permission_from_sandbox(Some(&sandbox));

        assert!(matches!(permission.fs_read, FsPolicy::None));
        assert!(matches!(permission.fs_write, FsPolicy::None));
        assert!(matches!(permission.shell, ShellPolicy::None));
        assert!(!permission.web_fetch);
        assert!(!permission.browser);
    }
}
