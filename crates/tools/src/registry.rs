use ccode_ports::{
    provider::ToolDefinition,
    tool::{ToolContext, ToolPort},
    PortError,
};
use serde_json::Value;
use std::sync::Arc;

pub struct ToolRegistry {
    tools: Vec<Arc<dyn ToolPort>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: Arc<dyn ToolPort>) {
        self.tools.push(tool);
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|t| t.definition()).collect()
    }

    pub async fn execute(
        &self,
        name: &str,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<String, PortError> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.name() == name)
            .ok_or_else(|| PortError::Tool(format!("unknown tool: {name}")))?;
        tool.execute(args, ctx).await
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
