use async_trait::async_trait;
use ccode_ports::{
    PortError,
    provider::{CompletionRequest, CompletionResponse, ProviderPort, ProviderStream},
};
use crate::openai_compat::OpenAiCompatClient;

pub struct OpenRouterAdapter {
    client: OpenAiCompatClient,
}

impl OpenRouterAdapter {
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        site_url: Option<String>,
        site_name: Option<String>,
    ) -> Self {
        let mut extra_headers = Vec::new();
        if let Some(url) = site_url {
            extra_headers.push(("HTTP-Referer".into(), url));
        }
        if let Some(name) = site_name {
            extra_headers.push(("X-Title".into(), name));
        }
        Self {
            client: OpenAiCompatClient::new(api_key, base_url, default_model, extra_headers),
        }
    }
}

#[async_trait]
impl ProviderPort for OpenRouterAdapter {
    fn name(&self) -> &str {
        "openrouter"
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
