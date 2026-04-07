use crate::openai_compat::OpenAiCompatAdapter;
use async_trait::async_trait;
use ccode_ports::provider::{
    LlmClient, LlmError, LlmRequest, LlmResponse, LlmStream, ProviderCapabilities,
};

pub struct LlamaCppAdapter {
    adapter: OpenAiCompatAdapter,
}

impl LlamaCppAdapter {
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            adapter: OpenAiCompatAdapter::new(
                "llamacpp",
                api_key,
                base_url,
                default_model,
                vec![],
                ProviderCapabilities {
                    vision: false,
                    context_window: None,
                },
            ),
        }
    }
}

#[async_trait]
impl LlmClient for LlamaCppAdapter {
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
