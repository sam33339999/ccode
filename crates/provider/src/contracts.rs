use async_trait::async_trait;
use futures_core::Stream;
use serde_json::Value;
use std::pin::Pin;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub input: String,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub model: String,
    pub output: String,
}

#[derive(Debug, Clone)]
pub struct StreamEvent {
    pub event_type: String,
    pub payload: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("authentication failed: {0}")]
    AuthError(String),
    #[error("rate limited")]
    RateLimited,
    #[error("model not available: {0}")]
    ModelNotAvailable(String),
    #[error("request too large: {0}")]
    RequestTooLarge(String),
    #[error("invalid response: {0}")]
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

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError>;
    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, LlmError>> + Send>>, LlmError>;
}
