use crate::openai_compat::OpenAiCompatAdapter;
use async_trait::async_trait;
use ccode_ports::provider::{
    LlmClient, LlmError, LlmRequest, LlmResponse, LlmStream, ProviderCapabilities,
};

pub struct ZhipuAdapter {
    adapter: OpenAiCompatAdapter,
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
            adapter: OpenAiCompatAdapter::new(
                "zhipu",
                api_key,
                base_url,
                default_model,
                extra_headers,
                ProviderCapabilities {
                    vision: false,
                    context_window: None,
                },
            ),
        }
    }
}

#[async_trait]
impl LlmClient for ZhipuAdapter {
    fn name(&self) -> &str {
        self.adapter.name()
    }

    fn default_model(&self) -> &str {
        self.adapter.default_model()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            vision: false,
            context_window: None,
        }
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
