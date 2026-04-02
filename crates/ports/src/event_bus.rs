use crate::PortError;
use async_trait::async_trait;
use ccode_domain::event::DomainEvent;

#[async_trait]
pub trait EventBusPort: Send + Sync {
    async fn publish(&self, event: DomainEvent) -> Result<(), PortError>;
}
