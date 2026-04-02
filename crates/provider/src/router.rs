use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use async_trait::async_trait;
use ccode_ports::{
    PortError,
    provider::{CompletionRequest, CompletionResponse, ProviderPort, ProviderStream},
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
    pub fn from_str(s: &str) -> Self {
        match s {
            "failover"    => Self::Failover,
            "round_robin" => Self::RoundRobin,
            _             => Self::Manual,
        }
    }
}

pub struct ProviderRouter {
    providers: Vec<Arc<dyn ProviderPort>>,
    strategy:  RoutingStrategy,
    rr_cursor: AtomicUsize,
}

impl ProviderRouter {
    pub fn new(providers: Vec<Arc<dyn ProviderPort>>, strategy: RoutingStrategy) -> Self {
        Self { providers, strategy, rr_cursor: AtomicUsize::new(0) }
    }

    fn pick_primary(&self) -> Option<&Arc<dyn ProviderPort>> {
        match self.strategy {
            RoutingStrategy::RoundRobin => {
                if self.providers.is_empty() { return None; }
                let idx = self.rr_cursor.fetch_add(1, Ordering::Relaxed) % self.providers.len();
                self.providers.get(idx)
            }
            _ => self.providers.first(),
        }
    }
}

#[async_trait]
impl ProviderPort for ProviderRouter {
    fn name(&self) -> &str { "router" }

    fn default_model(&self) -> &str {
        self.providers.first().map(|p| p.default_model()).unwrap_or("")
    }

    async fn health_check(&self) -> Result<(), PortError> {
        self.pick_primary()
            .ok_or_else(|| PortError::Provider("no providers configured".into()))?
            .health_check()
            .await
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, PortError> {
        match self.strategy {
            RoutingStrategy::Failover => {
                let mut last_err = PortError::Provider("no providers".into());
                for p in &self.providers {
                    match p.complete(req.clone()).await {
                        Ok(r)  => return Ok(r),
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
                    .ok_or_else(|| PortError::Provider("no providers configured".into()))?
                    .complete(req)
                    .await
            }
        }
    }

    async fn stream_complete(&self, req: CompletionRequest) -> Result<ProviderStream, PortError> {
        match self.strategy {
            RoutingStrategy::Failover => {
                let mut last_err = PortError::Provider("no providers".into());
                for p in &self.providers {
                    match p.stream_complete(req.clone()).await {
                        Ok(s)  => return Ok(s),
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
                    .ok_or_else(|| PortError::Provider("no providers configured".into()))?
                    .stream_complete(req)
                    .await
            }
        }
    }
}
