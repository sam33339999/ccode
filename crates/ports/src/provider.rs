use async_trait::async_trait;
use ccode_domain::message::Message;
use futures_core::Stream;
use std::pin::Pin;
use std::time::Duration;

// ── Tool definitions ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

// ── Request / Response types ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub messages: Vec<Message>,
    pub model: Option<String>, // None → use provider's default_model
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
    pub model: String,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Incremental content delta from the model.
    Delta { content: String },
    /// One or more tool calls produced by the model.
    ToolCallDone {
        tool_calls: Vec<ccode_domain::message::ToolCall>,
    },
    /// Stream finished. May include final usage stats.
    Done { usage: Option<TokenUsage> },
}

pub type LlmStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, LlmError>> + Send>>;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("authentication failed: {0}")]
    AuthError(String),
    #[error("rate limited")]
    RateLimited { retry_after_ms: Option<u64> },
    #[error("model not available: {0}")]
    ModelNotAvailable(String),
    #[error("request too large: {0}")]
    RequestTooLarge(String),
    #[error("provider returned invalid response: {0}")]
    InvalidResponse(String),
    #[error("stream interrupted: {0}")]
    StreamInterrupted(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("timeout after {0:?}")]
    Timeout(Duration),
    #[error("provider error ({status}): {message}")]
    ProviderError { status: u16, message: String },
}

// ── Provider port trait ────────────────────────────────────────────────────────

#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Provider identifier used in routing and logging.
    fn name(&self) -> &str;

    /// Default model for this provider.
    fn default_model(&self) -> &str;

    /// Liveness check.
    async fn health_check(&self) -> Result<(), LlmError>;

    /// Single-shot completion (waits for full response).
    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError>;

    /// Streaming completion — returns a stream of incremental events.
    async fn stream(&self, req: LlmRequest) -> Result<LlmStream, LlmError>;
}
