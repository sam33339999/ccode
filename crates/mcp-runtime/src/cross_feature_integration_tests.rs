use crate::contracts::{
    CapabilityLevel, ComputerUseLifecycle, DefaultMcpCapabilityPolicy, McpPolicyError,
    McpRuntimeError, McpServerRef, enforce_capability_policy,
};
use async_trait::async_trait;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct RecordingLifecycle {
    normal_cleanup_calls: AtomicUsize,
    interrupt_cleanup_calls: AtomicUsize,
}

#[async_trait]
impl ComputerUseLifecycle for RecordingLifecycle {
    async fn before_tool_call(&self) -> Result<(), McpRuntimeError> {
        Ok(())
    }

    async fn after_turn_cleanup(&self) -> Result<(), McpRuntimeError> {
        self.normal_cleanup_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn on_interrupt_cleanup(&self) -> Result<(), McpRuntimeError> {
        self.interrupt_cleanup_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

async fn run_turn(
    lifecycle: &dyn ComputerUseLifecycle,
    interrupted: bool,
) -> Result<(), McpRuntimeError> {
    lifecycle.before_tool_call().await?;
    if interrupted {
        lifecycle.on_interrupt_cleanup().await
    } else {
        lifecycle.after_turn_cleanup().await
    }
}

fn register_with_side_effect_counter(
    policy: &DefaultMcpCapabilityPolicy,
    server: &McpServerRef,
    gate_enabled: bool,
    side_effect_counter: &AtomicUsize,
) -> Result<CapabilityLevel, McpPolicyError> {
    let level = enforce_capability_policy(policy, server, gate_enabled)?;
    side_effect_counter.fetch_add(1, Ordering::SeqCst);
    Ok(level)
}

#[tokio::test]
async fn chicago_mcp_tool_runtime_cleanup_hooks_run_on_normal_completion_and_interruption() {
    let lifecycle = Arc::new(RecordingLifecycle::default());

    run_turn(lifecycle.as_ref(), false)
        .await
        .expect("normal turn should succeed");
    run_turn(lifecycle.as_ref(), true)
        .await
        .expect("interrupt turn should succeed");

    assert_eq!(
        lifecycle.normal_cleanup_calls.load(Ordering::SeqCst),
        1,
        "normal completion should invoke after_turn_cleanup once"
    );
    assert_eq!(
        lifecycle.interrupt_cleanup_calls.load(Ordering::SeqCst),
        1,
        "interrupt should invoke on_interrupt_cleanup once"
    );
}

#[test]
fn chicago_mcp_tool_runtime_privileged_capability_cannot_be_activated_via_generic_registration() {
    let policy = DefaultMcpCapabilityPolicy::new(true, true);
    let generic_server = McpServerRef::new("generic-mcp")
        .with_declared_capabilities([CapabilityLevel::PrivilegedComputerUse]);

    let negotiated = enforce_capability_policy(&policy, &generic_server, true)
        .expect("generic registration should remain standard");

    assert_eq!(
        negotiated,
        CapabilityLevel::Standard,
        "without explicit computer-use request, privileged capability must not activate"
    );
}

#[test]
fn chicago_mcp_tool_runtime_policy_failure_prevents_side_effects() {
    let policy = DefaultMcpCapabilityPolicy::new(false, true);
    let privileged_server = McpServerRef::new("desktop")
        .with_computer_use_requested(true)
        .with_declared_capabilities([CapabilityLevel::PrivilegedComputerUse]);
    let side_effect_counter = AtomicUsize::new(0);

    let err =
        register_with_side_effect_counter(&policy, &privileged_server, false, &side_effect_counter)
            .expect_err("policy failure should block registration");

    assert!(matches!(err, McpPolicyError::FeatureGateDisabled));
    assert_eq!(
        side_effect_counter.load(Ordering::SeqCst),
        0,
        "policy failures must not trigger side effects"
    );
}
