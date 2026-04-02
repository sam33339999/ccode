use async_trait::async_trait;
use ccode_domain::event::DomainEvent;
use crate::PortError;

#[async_trait]
pub trait EventBusPort: Send + Sync {
    async fn publish(&self, event: DomainEvent) -> Result<(), PortError>;
}
