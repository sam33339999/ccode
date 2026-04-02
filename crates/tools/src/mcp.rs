use async_trait::async_trait;
use ccode_mcp_runtime::{
    client::{JsonRpcMcpClient, McpClientError, McpToolDefinition},
    contracts::{
        enforce_capability_policy, CapabilityLevel, McpCapabilityPolicy, McpPolicyError,
        McpServerRef,
    },
    transport::{StdioTransport, TransportError},
};
use ccode_ports::{
    tool::{ToolContext, ToolPort},
    PortError,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::registry::MCP_DISCONNECTED_MARKER;

#[derive(Debug, Clone)]
pub struct McpServerLaunch {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub declared_capabilities: Vec<CapabilityLevel>,
    pub enable_computer_use: bool,
}

pub struct DiscoveredMcpTool {
    pub source: String,
    pub adapter: Arc<dyn ToolPort>,
}

pub async fn discover_mcp_tools(
    servers: &[McpServerLaunch],
    policy: &dyn McpCapabilityPolicy,
    chicago_mcp_feature_gate: bool,
) -> Result<Vec<DiscoveredMcpTool>, PortError> {
    let mut discovered = Vec::new();

    for server in servers {
        let server_ref = McpServerRef::new(server.name.clone())
            .with_computer_use_requested(server.enable_computer_use)
            .with_declared_capabilities(server.declared_capabilities.iter().copied());
        enforce_capability_policy(policy, &server_ref, chicago_mcp_feature_gate)
            .map_err(map_policy_error)?;

        let args: Vec<&str> = server.args.iter().map(String::as_str).collect();
        let transport = StdioTransport::spawn(&server.command, &args)
            .await
            .map_err(|e| {
                PortError::Tool(format!("mcp server `{}` spawn failed: {e}", server.name))
            })?;
        let mut client = JsonRpcMcpClient::new(Box::new(transport));
        let tools = client.list_tools().await.map_err(|e| {
            PortError::Tool(format!(
                "mcp server `{}` tools/list failed: {e}",
                server.name
            ))
        })?;
        let session = Arc::new(Mutex::new(client));
        let source = format!("mcp:{}", server.name);

        for tool in tools {
            let adapter = McpToolAdapter::new(server.name.clone(), tool, Arc::clone(&session));
            discovered.push(DiscoveredMcpTool {
                source: source.clone(),
                adapter: Arc::new(adapter),
            });
        }
    }

    Ok(discovered)
}

fn map_policy_error(err: McpPolicyError) -> PortError {
    PortError::Tool(format!("mcp policy rejected server: {err}"))
}

struct McpToolAdapter {
    server_name: String,
    tool_name: String,
    description: String,
    input_schema: Value,
    session: Arc<Mutex<JsonRpcMcpClient>>,
}

impl McpToolAdapter {
    fn new(
        server_name: String,
        tool: McpToolDefinition,
        session: Arc<Mutex<JsonRpcMcpClient>>,
    ) -> Self {
        Self {
            server_name,
            tool_name: tool.name,
            description: tool.description,
            input_schema: tool.input_schema,
            session,
        }
    }
}

#[async_trait]
impl ToolPort for McpToolAdapter {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.input_schema.clone()
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<String, PortError> {
        let mut client = self.session.lock().await;
        let result = client
            .call_tool(&self.tool_name, args)
            .await
            .map_err(|err| {
                if is_disconnect_error(&err) {
                    return PortError::Tool(format!(
                        "{MCP_DISCONNECTED_MARKER}server={}",
                        self.server_name
                    ));
                }
                PortError::Tool(format!(
                    "mcp server `{}` tool `{}` call failed: {err}",
                    self.server_name, self.tool_name
                ))
            })?;

        Ok(format_tool_result(&result))
    }
}

fn is_disconnect_error(err: &McpClientError) -> bool {
    matches!(
        err,
        McpClientError::Transport(TransportError::Closed)
            | McpClientError::Transport(TransportError::Io(_))
            | McpClientError::Transport(TransportError::Protocol(_))
    )
}

fn format_tool_result(result: &Value) -> String {
    if let Some(items) = result.get("content").and_then(Value::as_array) {
        let text = items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            return text;
        }
    }

    result.to_string()
}
