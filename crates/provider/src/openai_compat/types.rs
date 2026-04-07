use serde::{Deserialize, Serialize};

// ── Tool types ─────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(super) struct OpenAiToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(super) struct OpenAiTool {
    pub r#type: String, // always "function"
    pub function: OpenAiToolFunction,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(super) struct OpenAiToolCallFunction {
    pub name: String,
    pub arguments: String, // JSON string
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(super) struct OpenAiToolCall {
    pub id: String,
    pub r#type: String,
    pub function: OpenAiToolCallFunction,
}

// ── Outgoing request ───────────────────────────────────────────────────────────

#[derive(Serialize)]
pub(super) struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<OpenAiTool>,
    /// 有 tools 時送 "auto"，讓模型自行決定是否呼叫
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
}

/// OpenAI chat message — 根據 role 不同，content/tool_calls/tool_call_id 的組合不同：
/// - user/system:   content = Some(text)
/// - assistant:     content = Some(text) 或 None（純 tool call 時），tool_calls = Some([...])
/// - tool:          content = Some(result), tool_call_id = Some(id)
#[derive(Serialize)]
pub(super) struct ChatMessage {
    pub role: String,
    /// None 代表 null（assistant 只發工具呼叫時不含文字）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ChatMessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum ChatMessageContent {
    Text(String),
    Blocks(Vec<ChatContentBlock>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ChatContentBlock {
    Text { text: String },
    ImageUrl { image_url: ChatImageUrl },
}

#[derive(Serialize)]
pub(super) struct ChatImageUrl {
    pub url: String,
}

// ── Non-streaming response ─────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(super) struct ChatResponse {
    pub model: String,
    pub choices: Vec<ChatChoice>,
    pub usage: Option<UsageStats>,
}

#[derive(Deserialize)]
pub(super) struct ChatChoice {
    pub message: Option<ChatResponseMessage>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub(super) struct ChatResponseMessage {
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<OpenAiToolCall>>,
}

// ── Streaming chunk ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(super) struct StreamChunk {
    pub choices: Vec<StreamChoice>,
    pub usage: Option<UsageStats>,
}

#[derive(Deserialize)]
pub(super) struct StreamChoice {
    pub delta: DeltaContent,
    pub finish_reason: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct DeltaContent {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Deserialize)]
pub(super) struct StreamToolCallDelta {
    pub index: usize,
    pub id: Option<String>,
    pub function: Option<StreamToolCallFunctionDelta>,
}

#[derive(Deserialize)]
pub(super) struct StreamToolCallFunctionDelta {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

// ── Shared ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(super) struct UsageStats {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}
