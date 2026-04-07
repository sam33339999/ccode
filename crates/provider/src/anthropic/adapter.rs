use crate::anthropic_compat::AnthropicCompatClient;
use async_trait::async_trait;
use ccode_ports::provider::{
    LlmClient, LlmError, LlmRequest, LlmResponse, LlmStream, ProviderCapabilities,
};

pub struct AnthropicAdapter {
    client: AnthropicCompatClient,
    supports_vision: bool,
    context_window: Option<usize>,
}

impl AnthropicAdapter {
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self::new_with_capabilities(api_key, base_url, default_model, false, None)
    }

    pub fn new_with_capabilities(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        supports_vision: bool,
        context_window: Option<usize>,
    ) -> Self {
        Self {
            client: AnthropicCompatClient::new(api_key, base_url, default_model, supports_vision),
            supports_vision,
            context_window,
        }
    }

    /// Inject a custom `reqwest::Client` — used in acceptance tests to set short timeouts.
    pub fn new_for_test(
        http: reqwest::Client,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            client: AnthropicCompatClient::new_for_test(
                http,
                api_key,
                base_url,
                default_model,
                false,
            ),
            supports_vision: false,
            context_window: None,
        }
    }
}

#[async_trait]
impl LlmClient for AnthropicAdapter {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn default_model(&self) -> &str {
        &self.client.default_model
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            vision: self.supports_vision,
            context_window: self.context_window,
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
