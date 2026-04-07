use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ImageSource },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageSource {
    pub media_type: ImageMediaType,
    pub data: ImageData,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageMediaType {
    Jpeg,
    Png,
    Gif,
    Webp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ImageData {
    Base64(String),
    Url(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub system: Option<String>,
    pub stop_sequences: Vec<String>,
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: TokenUsage,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum StreamEvent {
    #[serde(rename = "content_delta")]
    ContentDelta { index: usize, text: String },
    #[serde(rename = "tool_call_delta")]
    ToolCallDelta {
        index: usize,
        id: String,
        name: String,
        input_json: String,
    },
    #[serde(rename = "message_start")]
    MessageStart,
    #[serde(rename = "message_complete")]
    MessageComplete {
        stop_reason: StopReason,
        usage: TokenUsage,
    },
}

pub mod constants {
    pub mod api_types {
        pub const CONTENT_BLOCK_TEXT: &str = "text";
        pub const CONTENT_BLOCK_IMAGE: &str = "image";
        pub const CONTENT_BLOCK_TOOL_USE: &str = "tool_use";
        pub const CONTENT_BLOCK_TOOL_RESULT: &str = "tool_result";
        pub const CONTENT_BLOCK_THINKING: &str = "thinking";
        pub const IMAGE_MEDIA_TYPE_JPEG: &str = "jpeg";
        pub const IMAGE_MEDIA_TYPE_PNG: &str = "png";
        pub const IMAGE_MEDIA_TYPE_GIF: &str = "gif";
        pub const IMAGE_MEDIA_TYPE_WEBP: &str = "webp";
        pub const IMAGE_DATA_BASE64: &str = "base64";
        pub const IMAGE_DATA_URL: &str = "url";

        pub const STOP_REASON_END_TURN: &str = "end_turn";
        pub const STOP_REASON_TOOL_USE: &str = "tool_use";
        pub const STOP_REASON_MAX_TOKENS: &str = "max_tokens";
        pub const STOP_REASON_STOP_SEQUENCE: &str = "stop_sequence";

        pub const ROLE_USER: &str = "user";
        pub const ROLE_ASSISTANT: &str = "assistant";
        pub const ROLE_SYSTEM: &str = "system";
    }

    pub mod config {
        pub const ENV_OPENAI_API_KEY: &str = "OPENAI_API_KEY";
        pub const ENV_ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
        pub const ENV_OPENROUTER_API_KEY: &str = "OPENROUTER_API_KEY";
        pub const ENV_ZHIPU_API_KEY: &str = "ZHIPU_API_KEY";

        pub const MODEL_ALIASES: [(&str, &str); 4] = [
            ("claude-sonnet", "claude-3-7-sonnet-latest"),
            ("claude-haiku", "claude-3-5-haiku-latest"),
            ("gpt-4o", "gpt-4o"),
            ("gpt-4.1-mini", "gpt-4.1-mini"),
        ];
    }

    pub mod core_domain {
        pub const ROLE_SYSTEM_ALLOWED_IN_REQUEST: bool = true;
        pub const STOP_REASON_REQUIRED_ON_RESPONSE: bool = true;

        pub const ROLE_INVARIANT: &str = "messages use only user/assistant/system roles";
        pub const STOP_REASON_INVARIANT: &str =
            "responses always include a stop reason from canonical set";
    }
}
