use serde::{Deserialize, Serialize};

// ── Tool types ─────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(super) struct AnthropicTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

// ── Outgoing request ───────────────────────────────────────────────────────────

#[derive(Serialize)]
pub(super) struct AnthropicRequest {
    pub model: String,
    /// Required by Anthropic API.
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<AnthropicMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AnthropicTool>,
}

/// Anthropic message — content 可為純字串或 content block 陣列。
/// 純文字訊息用字串（較簡潔），含 tool_use / tool_result 的訊息用陣列。
#[derive(Serialize, Deserialize)]
pub(super) struct AnthropicMessage {
    pub role: String,
    pub content: serde_json::Value,
}

// ── Non-streaming response ─────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(super) struct AnthropicResponse {
    pub model: String,
    pub content: Vec<AnthropicContent>,
    pub usage: AnthropicUsage,
}

#[derive(Deserialize)]
pub(super) struct AnthropicContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: Option<String>,
}

// ── Streaming events ───────────────────────────────────────────────────────────

/// Single envelope for all Anthropic SSE event payloads.
/// Unknown fields are ignored — only the relevant variants populate their fields.
#[derive(Deserialize)]
pub(super) struct AnthropicEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    /// Present on `content_block_delta`.
    pub delta: Option<AnthropicEventDelta>,
    /// Present on `message_start`.
    pub message: Option<AnthropicEventMessage>,
    /// Present on `message_delta` (contains output token count).
    pub usage: Option<AnthropicDeltaUsage>,
    /// Present on `content_block_start`.
    pub content_block: Option<AnthropicContentBlock>,
    /// Present on `content_block_start` / `content_block_stop` / `content_block_delta`.
    pub index: Option<usize>,
}

#[derive(Deserialize)]
pub(super) struct AnthropicContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    /// Present when block_type == "tool_use"
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct AnthropicEventDelta {
    #[serde(rename = "type")]
    pub delta_type: String,
    /// Populated when `delta_type == "text_delta"`.
    pub text: Option<String>,
    /// Populated when `delta_type == "input_json_delta"`.
    pub partial_json: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct AnthropicEventMessage {
    pub usage: Option<AnthropicUsage>,
}

// ── Shared ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Clone)]
pub(super) struct AnthropicUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Deserialize)]
pub(super) struct AnthropicDeltaUsage {
    pub output_tokens: u32,
}
