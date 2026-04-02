use ccode_domain::assistant_mode::AssistantMode;
use ccode_ports::{
    provider::ToolDefinition,
    tool::{ToolContext, ToolPort},
    PortError,
};
use serde_json::Value;
use std::sync::{Arc, RwLock};

pub(crate) const MCP_DISCONNECTED_MARKER: &str = "__MCP_DISCONNECTED__:";
const BRIEF_MODE_TOOL_ALLOWLIST: &[&str] =
    &["fs_glob", "fs_grep", "fs_list", "fs_read", "web_fetch"];

enum ModeAllowlist {
    All,
    Names(&'static [&'static str]),
}

impl ModeAllowlist {
    fn includes(&self, tool_name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Names(names) => names.contains(&tool_name),
        }
    }
}

struct ToolEntry {
    source: String,
    tool: Arc<dyn ToolPort>,
}

pub struct ToolRegistry {
    tools: RwLock<Vec<ToolEntry>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(Vec::new()),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn ToolPort>) {
        self.register_with_source("local", tool);
    }

    pub fn register_with_source(&self, source: impl Into<String>, tool: Arc<dyn ToolPort>) {
        let mut tools = self.tools.write().expect("tool registry poisoned");
        tools.push(ToolEntry {
            source: source.into(),
            tool,
        });
    }

    pub fn remove_by_source(&self, source: &str) {
        let mut tools = self.tools.write().expect("tool registry poisoned");
        tools.retain(|entry| entry.source != source);
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.definitions_for_mode(AssistantMode::Kairos)
    }

    pub fn definitions_for_mode(&self, mode: AssistantMode) -> Vec<ToolDefinition> {
        let allowlist = Self::allowlist_for_mode(mode);
        let tools = self.tools.read().expect("tool registry poisoned");
        tools
            .iter()
            .filter(|entry| allowlist.includes(entry.tool.name()))
            .map(|entry| entry.tool.definition())
            .collect()
    }

    pub async fn execute(
        &self,
        name: &str,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<String, PortError> {
        let entry = self
            .tools
            .read()
            .expect("tool registry poisoned")
            .iter()
            .find(|entry| entry.tool.name() == name)
            .map(|entry| (entry.source.clone(), Arc::clone(&entry.tool)))
            .ok_or_else(|| PortError::Tool(format!("unknown tool: {name}")))?;

        let (source, tool) = entry;
        let result = tool.execute(args, ctx).await;

        if source.starts_with("mcp:")
            && matches!(
                &result,
                Err(PortError::Tool(message)) if message.starts_with(MCP_DISCONNECTED_MARKER)
            )
        {
            self.remove_by_source(source.as_str());
        }

        result
    }

    const fn allowlist_for_mode(mode: AssistantMode) -> ModeAllowlist {
        match mode {
            AssistantMode::Kairos => ModeAllowlist::All,
            AssistantMode::KairosBrief => ModeAllowlist::Names(BRIEF_MODE_TOOL_ALLOWLIST),
            AssistantMode::KairosChannels => ModeAllowlist::All,
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ccode_domain::assistant_mode::AssistantMode;
    use ccode_ports::tool::Permission;
    use std::path::PathBuf;

    struct DisconnectingTool;
    struct NamedTool(&'static str);

    #[async_trait]
    impl ToolPort for DisconnectingTool {
        fn name(&self) -> &str {
            "mcp_echo"
        }

        fn description(&self) -> &str {
            "mock mcp tool"
        }

        fn parameters_schema(&self) -> Value {
            serde_json::json!({"type":"object"})
        }

        async fn execute(&self, _args: Value, _ctx: &ToolContext) -> Result<String, PortError> {
            Err(PortError::Tool(
                "__MCP_DISCONNECTED__:server=demo".to_string(),
            ))
        }
    }

    #[async_trait]
    impl ToolPort for NamedTool {
        fn name(&self) -> &str {
            self.0
        }

        fn description(&self) -> &str {
            "named tool"
        }

        fn parameters_schema(&self) -> Value {
            serde_json::json!({"type":"object"})
        }

        async fn execute(&self, _args: Value, _ctx: &ToolContext) -> Result<String, PortError> {
            Ok("ok".to_string())
        }
    }

    #[tokio::test]
    async fn disconnect_error_removes_tools_from_same_source() {
        let registry = ToolRegistry::new();
        registry.register_with_source("mcp:demo", Arc::new(DisconnectingTool));
        registry.register_with_source("mcp:demo", Arc::new(DisconnectingTool));
        assert_eq!(registry.definitions().len(), 2);

        let ctx = ToolContext {
            cwd: PathBuf::from("."),
            permission: Permission::default(),
        };
        let _ = registry
            .execute("mcp_echo", serde_json::json!({}), &ctx)
            .await;

        assert!(
            registry.definitions().is_empty(),
            "disconnect should remove all server tools"
        );
    }

    #[test]
    fn definitions_for_mode_hides_high_risk_tools_in_brief_mode() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(NamedTool("fs_read")));
        registry.register(Arc::new(NamedTool("shell")));
        registry.register(Arc::new(NamedTool("spawn_agent")));
        registry.register(Arc::new(NamedTool("web_fetch")));
        registry.register(Arc::new(NamedTool("mcp_custom_tool")));

        let names: Vec<String> = registry
            .definitions_for_mode(AssistantMode::KairosBrief)
            .into_iter()
            .map(|def| def.name)
            .collect();

        assert_eq!(names, vec!["fs_read", "web_fetch"]);
    }

    #[test]
    fn definitions_for_mode_is_deterministic_and_updates_immediately() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(NamedTool("fs_read")));
        registry.register(Arc::new(NamedTool("fs_write")));
        registry.register(Arc::new(NamedTool("shell")));
        registry.register(Arc::new(NamedTool("web_fetch")));

        let kairos_names: Vec<String> = registry
            .definitions_for_mode(AssistantMode::Kairos)
            .into_iter()
            .map(|def| def.name)
            .collect();
        let brief_names: Vec<String> = registry
            .definitions_for_mode(AssistantMode::KairosBrief)
            .into_iter()
            .map(|def| def.name)
            .collect();
        let channels_names: Vec<String> = registry
            .definitions_for_mode(AssistantMode::KairosChannels)
            .into_iter()
            .map(|def| def.name)
            .collect();

        assert_eq!(
            kairos_names,
            vec!["fs_read", "fs_write", "shell", "web_fetch"]
        );
        assert_eq!(brief_names, vec!["fs_read", "web_fetch"]);
        assert_eq!(
            channels_names,
            vec!["fs_read", "fs_write", "shell", "web_fetch"]
        );
    }
}
