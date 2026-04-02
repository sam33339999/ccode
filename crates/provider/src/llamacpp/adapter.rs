use crate::openai_compat::OpenAiCompatClient;
use async_trait::async_trait;
use ccode_ports::provider::{LlmClient, LlmError, LlmRequest, LlmResponse, LlmStream};

pub struct LlamaCppAdapter {
    client: OpenAiCompatClient,
}

impl LlamaCppAdapter {
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            client: OpenAiCompatClient::new(api_key, base_url, default_model, vec![]),
        }
    }
}

#[async_trait]
impl LlmClient for LlamaCppAdapter {
    fn name(&self) -> &str {
        "llamacpp"
    }

    fn default_model(&self) -> &str {
        &self.client.default_model
    }

    async fn health_check(&self) -> Result<(), LlmError> {
        self.client.health_check().await
    }

    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        self.client.complete(req).await
    }

    async fn stream(&self, req: LlmRequest) -> Result<LlmStream, LlmError> {
        self.client.stream(req).await
    }
}
