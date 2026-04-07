use ccode_config::schema::Config;
use ccode_ports::provider::LlmClient;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum FactoryError {
    #[error("provider '{0}' is not configured (missing api_key)")]
    NotConfigured(&'static str),
    #[error("unknown provider: {0}")]
    Unknown(String),
}

/// Build a provider adapter from config.
/// `name` must match a key in `config.providers` (e.g. `"openrouter"`).
pub fn build(name: &str, config: &Config) -> Result<Arc<dyn LlmClient>, FactoryError> {
    match name {
        #[cfg(feature = "provider-openrouter")]
        "openrouter" => {
            use crate::openrouter::OpenRouterAdapter;
            let cfg = config
                .providers
                .openrouter
                .as_ref()
                .cloned()
                .unwrap_or_default();
            let api_key = cfg
                .resolved_api_key()
                .ok_or(FactoryError::NotConfigured("openrouter"))?;
            Ok(Arc::new(OpenRouterAdapter::new(
                api_key,
                cfg.resolved_base_url(),
                cfg.resolved_default_model(),
                cfg.site_url.clone(),
                cfg.site_name.clone(),
                cfg.vision.unwrap_or(false),
                cfg.context_window,
            )))
        }
        #[cfg(feature = "provider-zhipu")]
        "zhipu" => {
            use crate::zhipu::ZhipuAdapter;
            let cfg = config.providers.zhipu.as_ref().cloned().unwrap_or_default();
            let api_key = cfg
                .resolved_api_key()
                .ok_or(FactoryError::NotConfigured("zhipu"))?;
            Ok(Arc::new(ZhipuAdapter::new(
                api_key,
                cfg.resolved_base_url(),
                cfg.resolved_default_model(),
                cfg.title.clone(),
            )))
        }
        #[cfg(feature = "provider-llamacpp")]
        "llamacpp" => {
            use crate::llamacpp::LlamaCppAdapter;
            let cfg = config
                .providers
                .llamacpp
                .as_ref()
                .cloned()
                .unwrap_or_default();
            Ok(Arc::new(LlamaCppAdapter::new(
                cfg.resolved_api_key(),
                cfg.resolved_base_url(),
                cfg.resolved_default_model(),
            )))
        }
        #[cfg(feature = "provider-anthropic")]
        "anthropic" => {
            use crate::anthropic::AnthropicAdapter;
            let cfg = config
                .providers
                .anthropic
                .as_ref()
                .cloned()
                .unwrap_or_default();
            let api_key = cfg
                .resolved_api_key()
                .ok_or(FactoryError::NotConfigured("anthropic"))?;
            Ok(Arc::new(AnthropicAdapter::new_with_capabilities(
                api_key,
                cfg.resolved_base_url(),
                cfg.resolved_default_model(),
                cfg.vision.unwrap_or(false),
                cfg.context_window,
            )))
        }
        other => Err(FactoryError::Unknown(other.into())),
    }
}

/// Build the active provider from config.
///
/// - `manual`      → single provider named by `routing.default_provider`
/// - `failover`    → all configured providers in config order, try each on error
/// - `round_robin` → all configured providers, rotated per request
pub fn build_default(config: &Config) -> Result<Arc<dyn LlmClient>, FactoryError> {
    use crate::router::{ProviderRouter, RoutingStrategy};

    let strategy = RoutingStrategy::from_config_value(&config.routing.strategy);

    if strategy == RoutingStrategy::Manual {
        let name = config
            .routing
            .default_provider
            .as_deref()
            .unwrap_or("openrouter");
        return build(name, config);
    }

    // Build all providers that are configured
    let candidates = ["openrouter", "anthropic", "zhipu", "llamacpp"];
    let mut providers: Vec<Arc<dyn LlmClient>> = Vec::new();

    // Put default_provider first if specified
    if let Some(default) = config.routing.default_provider.as_deref()
        && let Ok(p) = build(default, config)
    {
        providers.push(p);
    }
    for name in candidates {
        if Some(name) == config.routing.default_provider.as_deref() {
            continue; // already added
        }
        if let Ok(p) = build(name, config) {
            providers.push(p);
        }
    }

    if providers.is_empty() {
        return Err(FactoryError::NotConfigured("no providers available"));
    }

    Ok(Arc::new(ProviderRouter::new(providers, strategy)))
}
