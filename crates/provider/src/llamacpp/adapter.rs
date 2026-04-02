use crate::openai_compat::OpenAiCompatClient;
use async_trait::async_trait;
use ccode_ports::{
    PortError,
    provider::{CompletionRequest, CompletionResponse, ProviderPort, ProviderStream},
};

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
impl ProviderPort for LlamaCppAdapter {
    fn name(&self) -> &str {
        "llamacpp"
    }

    fn default_model(&self) -> &str {
        &self.client.default_model
    }

    async fn health_check(&self) -> Result<(), PortError> {
        self.client.health_check().await
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, PortError> {
        self.client.complete(req).await
    }

    async fn stream_complete(&self, req: CompletionRequest) -> Result<ProviderStream, PortError> {
        self.client.stream_complete(req).await
    }
}
