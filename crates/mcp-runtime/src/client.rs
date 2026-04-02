use serde_json::{Value, json};

use crate::transport::{McpTransport, TransportError};

#[derive(Debug, Clone, PartialEq)]
pub struct McpToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum McpClientError {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("rpc error ({code}): {message}")]
    Rpc { code: i64, message: String },
}

pub struct JsonRpcMcpClient {
    transport: Box<dyn McpTransport>,
    next_id: u64,
}

impl JsonRpcMcpClient {
    pub fn new(transport: Box<dyn McpTransport>) -> Self {
        Self {
            transport,
            next_id: 1,
        }
    }

    pub async fn list_tools(&mut self) -> Result<Vec<McpToolDefinition>, McpClientError> {
        let result = self.call("tools/list", json!({})).await?;
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| McpClientError::Protocol("tools/list result missing `tools`".into()))?;

        tools
            .iter()
            .map(|tool| {
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| McpClientError::Protocol("tool missing `name`".into()))?
                    .to_string();
                let description = tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let input_schema = tool
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({"type":"object"}));

                Ok(McpToolDefinition {
                    name,
                    description,
                    input_schema,
                })
            })
            .collect()
    }

    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Result<Value, McpClientError> {
        self.call(
            "tools/call",
            json!({
                "name": name,
                "arguments": arguments
            }),
        )
        .await
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value, McpClientError> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        self.transport.send(&request).await?;

        loop {
            let message = self.transport.receive().await?;

            if message.get("id").and_then(Value::as_u64) != Some(id) {
                // Ignore notifications or responses for other requests.
                continue;
            }

            if let Some(error) = message.get("error") {
                let code = error.get("code").and_then(Value::as_i64).unwrap_or(-1);
                let msg = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown RPC error")
                    .to_string();
                return Err(McpClientError::Rpc { code, message: msg });
            }

            return message
                .get("result")
                .cloned()
                .ok_or_else(|| McpClientError::Protocol("response missing `result`".into()));
        }
    }
}
