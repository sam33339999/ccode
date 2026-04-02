use crate::openai_compat::OpenAiCompatClient;
use async_trait::async_trait;
use ccode_ports::provider::{LlmClient, LlmError, LlmRequest, LlmResponse, LlmStream};

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
impl LlmClient for ZhipuAdapter {
    fn name(&self) -> &str {
        "zhipu"
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
