use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub role: Role,
    pub content: String,
    /// Unix timestamp (ms)
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn new(
        id: impl Into<String>,
        role: Role,
        content: impl Into<String>,
        created_at: u64,
    ) -> Self {
        Self {
            id: MessageId(id.into()),
            role,
            content: content.into(),
            created_at,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn new_tool_result(
        id: impl Into<String>,
        tool_call_id: impl Into<String>,
        content: impl Into<String>,
        created_at: u64,
    ) -> Self {
        Self {
            id: MessageId(id.into()),
            role: Role::Tool,
            content: content.into(),
            created_at,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}
