use crate::anthropic_compat::AnthropicCompatClient;
use async_trait::async_trait;
use ccode_ports::provider::{LlmClient, LlmError, LlmRequest, LlmResponse, LlmStream};

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

    /// Inject a custom `reqwest::Client` — used in acceptance tests to set short timeouts.
    pub fn new_for_test(
        http: reqwest::Client,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            client: AnthropicCompatClient::new_for_test(http, api_key, base_url, default_model),
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
