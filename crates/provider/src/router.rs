use async_trait::async_trait;
use ccode_ports::provider::{
    LlmClient, LlmError, LlmRequest, LlmResponse, LlmStream, ProviderCapabilities,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug, Clone, PartialEq)]
pub enum RoutingStrategy {
    /// Always use the first (default) provider.
    Manual,
    /// Try providers in order; advance to the next on error.
    Failover,
    /// Distribute requests round-robin across all providers.
    RoundRobin,
}

impl RoutingStrategy {
    pub fn from_config_value(s: &str) -> Self {
        match s {
            "failover" => Self::Failover,
            "round_robin" => Self::RoundRobin,
            _ => Self::Manual,
        }
    }
}

pub struct ProviderRouter {
    providers: Vec<Arc<dyn LlmClient>>,
    strategy: RoutingStrategy,
    rr_cursor: AtomicUsize,
}

impl ProviderRouter {
    pub fn new(providers: Vec<Arc<dyn LlmClient>>, strategy: RoutingStrategy) -> Self {
        Self {
            providers,
            strategy,
            rr_cursor: AtomicUsize::new(0),
        }
    }

    fn pick_primary(&self) -> Option<&Arc<dyn LlmClient>> {
        match self.strategy {
            RoutingStrategy::RoundRobin => {
                if self.providers.is_empty() {
                    return None;
                }
                let idx = self.rr_cursor.fetch_add(1, Ordering::Relaxed) % self.providers.len();
                self.providers.get(idx)
            }
            _ => self.providers.first(),
        }
    }
}

#[async_trait]
impl LlmClient for ProviderRouter {
    fn name(&self) -> &str {
        "router"
    }

    fn default_model(&self) -> &str {
        self.providers
            .first()
            .map(|p| p.default_model())
            .unwrap_or("")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        let mut iter = self.providers.iter().map(|p| p.capabilities());
        let Some(first) = iter.next() else {
            return ProviderCapabilities {
                vision: false,
                context_window: None,
            };
        };

        iter.fold(first, |acc, caps| ProviderCapabilities {
            vision: acc.vision && caps.vision,
            context_window: match (acc.context_window, caps.context_window) {
                (Some(a), Some(b)) => Some(a.min(b)),
                _ => None,
            },
        })
    }

    async fn health_check(&self) -> Result<(), LlmError> {
        self.pick_primary()
            .ok_or_else(|| LlmError::Network("no providers configured".into()))?
            .health_check()
            .await
    }

    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        match self.strategy {
            RoutingStrategy::Failover => {
                let mut last_err = LlmError::Network("no providers".into());
                for p in &self.providers {
                    match p.complete(req.clone()).await {
                        Ok(r) => return Ok(r),
                        Err(e) => {
                            tracing::warn!("[router] provider {} failed: {e}", p.name());
                            last_err = e;
                        }
                    }
                }
                Err(last_err)
            }
            _ => {
                self.pick_primary()
                    .ok_or_else(|| LlmError::Network("no providers configured".into()))?
                    .complete(req)
                    .await
            }
        }
    }

    async fn stream(&self, req: LlmRequest) -> Result<LlmStream, LlmError> {
        match self.strategy {
            RoutingStrategy::Failover => {
                let mut last_err = LlmError::Network("no providers".into());
                for p in &self.providers {
                    match p.stream(req.clone()).await {
                        Ok(s) => return Ok(s),
                        Err(e) => {
                            tracing::warn!("[router] provider {} failed: {e}", p.name());
                            last_err = e;
                        }
                    }
                }
                Err(last_err)
            }
            _ => {
                self.pick_primary()
                    .ok_or_else(|| LlmError::Network("no providers configured".into()))?
                    .stream(req)
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ccode_ports::provider::{ProviderCapabilities, TokenUsage};

    struct MockClient {
        name: &'static str,
        caps: ProviderCapabilities,
    }

    #[async_trait]
    impl LlmClient for MockClient {
        fn name(&self) -> &str {
            self.name
        }

        fn default_model(&self) -> &str {
            "mock-model"
        }

        fn capabilities(&self) -> ProviderCapabilities {
            self.caps
        }

        async fn health_check(&self) -> Result<(), LlmError> {
            Ok(())
        }

        async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
            Ok(LlmResponse {
                content: String::new(),
                model: "mock-model".into(),
                usage: Some(TokenUsage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                }),
            })
        }

        async fn stream(&self, _req: LlmRequest) -> Result<LlmStream, LlmError> {
            Err(LlmError::Network("unused in tests".into()))
        }
    }

    #[test]
    fn capabilities_vision_is_false_if_any_provider_disables_it() {
        let providers: Vec<Arc<dyn LlmClient>> = vec![
            Arc::new(MockClient {
                name: "a",
                caps: ProviderCapabilities {
                    vision: true,
                    context_window: Some(200_000),
                },
            }),
            Arc::new(MockClient {
                name: "b",
                caps: ProviderCapabilities {
                    vision: false,
                    context_window: Some(128_000),
                },
            }),
        ];

        let router = ProviderRouter::new(providers, RoutingStrategy::Manual);
        let caps = router.capabilities();
        assert!(!caps.vision);
        assert_eq!(caps.context_window, Some(128_000));
    }

    #[test]
    fn capabilities_context_window_is_none_if_any_provider_unknown() {
        let providers: Vec<Arc<dyn LlmClient>> = vec![
            Arc::new(MockClient {
                name: "a",
                caps: ProviderCapabilities {
                    vision: true,
                    context_window: Some(128_000),
                },
            }),
            Arc::new(MockClient {
                name: "b",
                caps: ProviderCapabilities {
                    vision: true,
                    context_window: None,
                },
            }),
        ];

        let router = ProviderRouter::new(providers, RoutingStrategy::Manual);
        let caps = router.capabilities();
        assert!(caps.vision);
        assert_eq!(caps.context_window, None);
    }
}
