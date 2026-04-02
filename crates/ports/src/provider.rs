use std::pin::Pin;
use async_trait::async_trait;
use futures_core::Stream;
use ccode_domain::message::Message;
use crate::PortError;

// ── Tool definitions ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

// ── Request / Response types ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub messages: Vec<Message>,
    pub model: Option<String>, // None → use provider's default_model
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone)]
pub struct CompletionResponse {
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
    ToolCallDone { tool_calls: Vec<ccode_domain::message::ToolCall> },
    /// Stream finished. May include final usage stats.
    Done { usage: Option<TokenUsage> },
}

pub type ProviderStream =
    Pin<Box<dyn Stream<Item = Result<StreamEvent, PortError>> + Send>>;

// ── Provider port trait ────────────────────────────────────────────────────────

#[async_trait]
pub trait ProviderPort: Send + Sync {
    /// Provider identifier used in routing and logging.
    fn name(&self) -> &str;

    /// Default model for this provider.
    fn default_model(&self) -> &str;

    /// Liveness check.
    async fn health_check(&self) -> Result<(), PortError>;

    /// Single-shot completion (waits for full response).
    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<CompletionResponse, PortError>;

    /// Streaming completion — returns a stream of incremental events.
    async fn stream_complete(
        &self,
        req: CompletionRequest,
    ) -> Result<ProviderStream, PortError>;
}
