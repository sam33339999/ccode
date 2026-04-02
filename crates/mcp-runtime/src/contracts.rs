use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct McpServerRef {
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityLevel {
    Standard,
    PrivilegedComputerUse,
}

#[derive(Debug, thiserror::Error)]
pub enum McpPolicyError {
    #[error("reserved server name")]
    ReservedServerName,
    #[error("feature gate disabled")]
    FeatureGateDisabled,
    #[error("privileged capability denied")]
    PrivilegedCapabilityDenied,
}

#[derive(Debug, thiserror::Error)]
pub enum McpRuntimeError {
    #[error("transport error")]
    TransportError,
    #[error("invalid tool payload")]
    InvalidToolPayload,
    #[error("cleanup failed")]
    CleanupFailed,
}

pub trait McpCapabilityPolicy: Send + Sync {
    fn validate_server_name(&self, name: &str) -> Result<(), McpPolicyError>;
    fn capability_level(&self, server: &McpServerRef) -> CapabilityLevel;
}

#[async_trait]
pub trait ComputerUseLifecycle: Send + Sync {
    async fn before_tool_call(&self) -> Result<(), McpRuntimeError>;
    async fn after_turn_cleanup(&self) -> Result<(), McpRuntimeError>;
    async fn on_interrupt_cleanup(&self) -> Result<(), McpRuntimeError>;
}
