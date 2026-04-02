use async_trait::async_trait;
use serde_json::json;
use std::collections::VecDeque;

use crate::client::JsonRpcMcpClient;
use crate::transport::{McpTransport, TransportError};

struct MockTransport {
    sent: Vec<serde_json::Value>,
    recv: VecDeque<Result<serde_json::Value, TransportError>>,
}

impl MockTransport {
    fn with_responses(responses: Vec<Result<serde_json::Value, TransportError>>) -> Self {
        Self {
            sent: Vec::new(),
            recv: responses.into(),
        }
    }
}

#[async_trait]
impl McpTransport for MockTransport {
    async fn send(&mut self, message: &serde_json::Value) -> Result<(), TransportError> {
        self.sent.push(message.clone());
        Ok(())
    }

    async fn receive(&mut self) -> Result<serde_json::Value, TransportError> {
        self.recv
            .pop_front()
            .expect("mock must have queued response")
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        Ok(())
    }
}

#[tokio::test]
async fn list_tools_parses_tool_definitions() {
    let response = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "tools": [
                {
                    "name": "read_file",
                    "description": "Read file",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" }
                        },
                        "required": ["path"]
                    }
                }
            ]
        }
    });
    let mock = MockTransport::with_responses(vec![Ok(response)]);
    let mut client = JsonRpcMcpClient::new(Box::new(mock));

    let tools = client
        .list_tools()
        .await
        .expect("tools/list should succeed");

    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "read_file");
    assert_eq!(tools[0].description, "Read file");
    assert_eq!(tools[0].input_schema["type"], "object");
}

#[tokio::test]
async fn call_tool_returns_result_payload() {
    let response = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "content": [
                { "type": "text", "text": "hello" }
            ]
        }
    });
    let mock = MockTransport::with_responses(vec![Ok(response)]);
    let mut client = JsonRpcMcpClient::new(Box::new(mock));

    let result = client
        .call_tool("echo", json!({"message":"hello"}))
        .await
        .expect("tools/call should succeed");

    assert_eq!(
        result["content"][0]["text"].as_str(),
        Some("hello"),
        "result payload should flow through"
    );
}
