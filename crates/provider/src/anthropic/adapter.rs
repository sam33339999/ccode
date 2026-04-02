use async_trait::async_trait;
use ccode_ports::{
    PortError,
    provider::{CompletionRequest, CompletionResponse, ProviderPort, ProviderStream},
};
use crate::anthropic_compat::AnthropicCompatClient;

pub struct AnthropicAdapter {
    client: AnthropicCompatClient,
}

impl AnthropicAdapter {
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            client: AnthropicCompatClient::new(api_key, base_url, default_model),
        }
    }
}

#[async_trait]
impl ProviderPort for AnthropicAdapter {
    fn name(&self) -> &str {
        "anthropic"
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
