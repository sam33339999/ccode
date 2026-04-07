mod client;
mod types;

use async_trait::async_trait;
use ccode_ports::provider::{
    LlmClient, LlmError, LlmRequest, LlmResponse, LlmStream, ProviderCapabilities,
};

pub use client::OpenAiCompatClient;

pub struct OpenAiCompatAdapter {
    name: String,
    client: OpenAiCompatClient,
}

impl OpenAiCompatAdapter {
    pub fn new(
        name: impl Into<String>,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        extra_headers: Vec<(String, String)>,
    ) -> Self {
        Self {
            name: name.into(),
            client: OpenAiCompatClient::new(api_key, base_url, default_model, extra_headers),
        }
    }
}

#[async_trait]
impl LlmClient for OpenAiCompatAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn default_model(&self) -> &str {
        &self.client.default_model
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            vision: false,
            context_window: None,
        }
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
