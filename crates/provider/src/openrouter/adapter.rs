use crate::openai_compat::OpenAiCompatAdapter;
use async_trait::async_trait;
use ccode_ports::provider::{LlmClient, LlmError, LlmRequest, LlmResponse, LlmStream};

pub struct OpenRouterAdapter {
    adapter: OpenAiCompatAdapter,
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
            adapter: OpenAiCompatAdapter::new(
                "openrouter",
                api_key,
                base_url,
                default_model,
                extra_headers,
            ),
        }
    }
}

#[async_trait]
impl LlmClient for OpenRouterAdapter {
    fn name(&self) -> &str {
        self.adapter.name()
    }

    fn default_model(&self) -> &str {
        self.adapter.default_model()
    }

    async fn health_check(&self) -> Result<(), LlmError> {
        self.adapter.health_check().await
    }

    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        self.adapter.complete(req).await
    }

    async fn stream(&self, req: LlmRequest) -> Result<LlmStream, LlmError> {
        self.adapter.stream(req).await
    }
}
