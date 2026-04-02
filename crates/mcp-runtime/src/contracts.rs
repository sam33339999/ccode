use async_trait::async_trait;
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct McpServerRef {
    pub name: String,
    pub computer_use_requested: bool,
    declared_capabilities: BTreeSet<CapabilityLevel>,
}

impl McpServerRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            computer_use_requested: false,
            declared_capabilities: BTreeSet::from([CapabilityLevel::Standard]),
        }
    }

    pub fn with_computer_use_requested(mut self, requested: bool) -> Self {
        self.computer_use_requested = requested;
        self
    }

    pub fn with_declared_capabilities(
        mut self,
        capabilities: impl IntoIterator<Item = CapabilityLevel>,
    ) -> Self {
        self.declared_capabilities = capabilities.into_iter().collect();
        self
    }

    pub fn supports(&self, capability: CapabilityLevel) -> bool {
        self.declared_capabilities.contains(&capability)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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
    #[error("transport error")]
    TransportError,
    #[error("invalid tool payload")]
    InvalidToolPayload,
    #[error("cleanup failed")]
    CleanupFailed,
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

impl From<McpRuntimeError> for McpPolicyError {
    fn from(value: McpRuntimeError) -> Self {
        match value {
            McpRuntimeError::TransportError => Self::TransportError,
            McpRuntimeError::InvalidToolPayload => Self::InvalidToolPayload,
            McpRuntimeError::CleanupFailed => Self::CleanupFailed,
        }
    }
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

pub struct DefaultMcpCapabilityPolicy {
    reserved_server_names: BTreeSet<String>,
    allow_privileged_computer_use: bool,
}

impl DefaultMcpCapabilityPolicy {
    pub fn new(_chicago_mcp_feature_gate: bool, allow_privileged_computer_use: bool) -> Self {
        let mut reserved_server_names = BTreeSet::new();
        reserved_server_names.insert("computer".to_string());
        Self {
            reserved_server_names,
            allow_privileged_computer_use,
        }
    }
}

impl McpCapabilityPolicy for DefaultMcpCapabilityPolicy {
    fn validate_server_name(&self, name: &str) -> Result<(), McpPolicyError> {
        let lowered = name.to_ascii_lowercase();
        if self.reserved_server_names.contains(&lowered) {
            return Err(McpPolicyError::ReservedServerName);
        }
        Ok(())
    }

    fn capability_level(&self, server: &McpServerRef) -> CapabilityLevel {
        if server.computer_use_requested && self.allow_privileged_computer_use {
            CapabilityLevel::PrivilegedComputerUse
        } else {
            CapabilityLevel::Standard
        }
    }
}

pub fn enforce_capability_policy(
    policy: &dyn McpCapabilityPolicy,
    server: &McpServerRef,
    chicago_mcp_feature_gate: bool,
) -> Result<CapabilityLevel, McpPolicyError> {
    policy.validate_server_name(&server.name)?;

    if !server.computer_use_requested {
        return Ok(CapabilityLevel::Standard);
    }

    if !chicago_mcp_feature_gate {
        return Err(McpPolicyError::FeatureGateDisabled);
    }

    if !server.supports(CapabilityLevel::PrivilegedComputerUse) {
        return Err(McpPolicyError::PrivilegedCapabilityDenied);
    }

    let negotiated = policy.capability_level(server);
    if negotiated != CapabilityLevel::PrivilegedComputerUse {
        return Err(McpPolicyError::PrivilegedCapabilityDenied);
    }

    Ok(negotiated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_name_is_rejected() {
        let policy = DefaultMcpCapabilityPolicy::new(true, false);
        let server =
            McpServerRef::new("computer").with_declared_capabilities([CapabilityLevel::Standard]);
        let err =
            enforce_capability_policy(&policy, &server, true).expect_err("must reject reserved");
        assert!(matches!(err, McpPolicyError::ReservedServerName));
    }

    #[test]
    fn privileged_requires_gate_and_policy_pass() {
        let policy = DefaultMcpCapabilityPolicy::new(false, true);
        let server = McpServerRef::new("desktop")
            .with_computer_use_requested(true)
            .with_declared_capabilities([CapabilityLevel::PrivilegedComputerUse]);
        let err = enforce_capability_policy(&policy, &server, false)
            .expect_err("must fail when feature gate is disabled");
        assert!(matches!(err, McpPolicyError::FeatureGateDisabled));
    }

    #[test]
    fn capability_negotiation_only_enables_declared_capabilities() {
        let policy = DefaultMcpCapabilityPolicy::new(true, true);
        let server = McpServerRef::new("desktop").with_computer_use_requested(true);
        let err = enforce_capability_policy(&policy, &server, true)
            .expect_err("must fail when server does not declare support");
        assert!(matches!(err, McpPolicyError::PrivilegedCapabilityDenied));
    }
}
