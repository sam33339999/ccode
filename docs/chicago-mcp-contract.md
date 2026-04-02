# CHICAGO_MCP Contract Spec (Rust)

## 1. Objective

Define Rust contracts for computer-use MCP while isolating high-privilege behavior from generic MCP runtime.

## 2. Evidence from Current Code

1. `entrypoints/cli.tsx:86` `--computer-use-mcp` fast path.
2. `services/mcp/client.ts:241` lazy wrapper under `feature('CHICAGO_MCP')`.
3. `services/mcp/config.ts:641` reserved-name policy when CHICAGO_MCP enabled.
4. `services/mcp/config.ts:1512` built-in default disabled behavior.
5. `query.ts:1033` turn-end cleanup on interrupt path.
6. `query.ts:1489` cleanup on normal path.
7. `utils/computerUse/wrapper.tsx` wrapper-level gate and call override semantics.
8. `utils/computerUse/cleanup.ts` cleanup contract.

## 3. Rust Boundary Mapping

1. `crates/api-types`: MCP tool request/response and capability metadata.
2. `crates/mcp-runtime`: protocol transport and server lifecycle.
3. `crates/tool-runtime`: tool registration/dispatch pipeline.
4. `crates/platform`: native lock/host adapter/process boundaries.
5. `crates/config`: gate keys, reserved-name policy flags.

## 4. Core Contracts (Rust Sketch)

```rust
#[derive(Debug, Clone)]
pub struct McpServerRef { pub name: String }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityLevel { Standard, PrivilegedComputerUse }

#[async_trait::async_trait]
pub trait McpCapabilityPolicy {
    fn validate_server_name(&self, name: &str) -> Result<(), McpPolicyError>;
    fn capability_level(&self, server: &McpServerRef) -> CapabilityLevel;
}

#[async_trait::async_trait]
pub trait ComputerUseLifecycle {
    async fn before_tool_call(&self) -> Result<(), McpRuntimeError>;
    async fn after_turn_cleanup(&self) -> Result<(), McpRuntimeError>;
    async fn on_interrupt_cleanup(&self) -> Result<(), McpRuntimeError>;
}
```

## 5. Error Taxonomy

1. `ReservedServerName`
2. `FeatureGateDisabled`
3. `PrivilegedCapabilityDenied`
4. `TransportError`
5. `InvalidToolPayload`
6. `CleanupFailed` (non-fatal in interrupt path)

## 6. Policy Rules

1. Reserved names are hard-rejected before persistence.
2. Privileged computer-use adapter requires gate + policy pass.
3. Cleanup must run in both normal and interrupt paths.
4. Cleanup failures are observable but do not crash the main turn path.

## 7. Constants Classification

1. `api-types`: MCP event labels and capability enums.
2. `config`: `CHICAGO_MCP` related gate keys and defaults.
3. `core-domain`: privileged transition invariants.
4. `mcp-runtime`/`platform`: wrapper and adapter internals.
5. `ui-tui`: display messages only.

## 8. Acceptance Matrix

### Contract tests

1. Reserved names rejected with `ReservedServerName`.
2. Privileged capability denied without gate/policy.
3. Cleanup API is called for both normal and interrupt exits.

### Integration tests

1. MCP server registration + tool dispatch happy path.
2. Interrupt during tool call triggers cleanup once.
3. Cleanup exception does not poison subsequent turns.

### CLI behavior tests

1. `--computer-use-mcp` routes to privileged adapter path.
2. Error output exposes class, not sensitive native details.
3. Disabled gate path gives deterministic reason.

