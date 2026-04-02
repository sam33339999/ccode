use async_trait::async_trait;
use ccode_ports::{
    PortError,
    provider::{CompletionRequest, CompletionResponse, ProviderPort, ProviderStream},
};
use crate::openai_compat::OpenAiCompatClient;

pub struct ZhipuAdapter {
    client: OpenAiCompatClient,
}

impl ZhipuAdapter {
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        title: Option<String>,
    ) -> Self {
        let mut extra_headers = Vec::new();
        if let Some(t) = title {
            extra_headers.push(("X-Title".into(), t));
        }
        Self {
            client: OpenAiCompatClient::new(api_key, base_url, default_model, extra_headers),
        }
    }
}

#[async_trait]
impl ProviderPort for ZhipuAdapter {
    fn name(&self) -> &str {
        "zhipu"
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
