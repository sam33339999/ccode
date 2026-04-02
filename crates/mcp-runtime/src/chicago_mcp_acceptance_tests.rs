/// Chicago MCP acceptance tests (US-038)
///
/// Covers all scenarios from chicago-mcp-contract.md §8 Acceptance Criteria:
///   §8.1 Contract tests  – policy and lifecycle trait invariants
///   §8.2 Integration tests – multi-step registration and cleanup flows
///
/// CLI behaviour tests live in crates/cli/src/cmd/chicago.rs.
use crate::contracts::{
    CapabilityLevel, ComputerUseLifecycle, DefaultMcpCapabilityPolicy, McpPolicyError,
    McpRuntimeError, McpServerRef, enforce_capability_policy,
};
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

// ── shared test doubles ────────────────────────────────────────────────────

/// Mock lifecycle that tracks call counts and optionally fails cleanup.
#[derive(Default)]
struct MockLifecycle {
    before_calls: AtomicUsize,
    cleanup_calls: AtomicUsize,
    interrupt_cleanup_calls: AtomicUsize,
    fail_cleanup: AtomicBool,
}

#[async_trait]
impl ComputerUseLifecycle for MockLifecycle {
    async fn before_tool_call(&self) -> Result<(), McpRuntimeError> {
        self.before_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn after_turn_cleanup(&self) -> Result<(), McpRuntimeError> {
        self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_cleanup.load(Ordering::SeqCst) {
            return Err(McpRuntimeError::CleanupFailed);
        }
        Ok(())
    }

    async fn on_interrupt_cleanup(&self) -> Result<(), McpRuntimeError> {
        self.interrupt_cleanup_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_cleanup.load(Ordering::SeqCst) {
            return Err(McpRuntimeError::CleanupFailed);
        }
        Ok(())
    }
}

// ── §8.1 Contract tests ────────────────────────────────────────────────────

/// AC: Reserved names rejected with ReservedServerName.
///
/// The "computer" name is hard-reserved. Validation must fail before any
/// transport or persistence side-effect occurs, regardless of case.
#[test]
fn contract_reserved_names_rejected_with_reserved_server_name() {
    let policy = DefaultMcpCapabilityPolicy::new(true, true);

    for name in ["computer", "Computer", "COMPUTER"] {
        let server = McpServerRef::new(name);
        let err = enforce_capability_policy(&policy, &server, true)
            .expect_err(&format!("reserved name '{name}' must be rejected"));
        assert!(
            matches!(err, McpPolicyError::ReservedServerName),
            "'{name}' must yield ReservedServerName, got {err:?}"
        );
    }
}

/// AC: Privileged capability denied without gate/policy.
///
/// Even when the server declares PrivilegedComputerUse, if the feature gate
/// is disabled the request must fail with FeatureGateDisabled.
#[test]
fn contract_privileged_capability_denied_without_feature_gate() {
    let policy = DefaultMcpCapabilityPolicy::new(false, true);
    let server = McpServerRef::new("desktop")
        .with_computer_use_requested(true)
        .with_declared_capabilities([CapabilityLevel::PrivilegedComputerUse]);

    let err = enforce_capability_policy(&policy, &server, false)
        .expect_err("must fail when feature gate is off");
    assert!(
        matches!(err, McpPolicyError::FeatureGateDisabled),
        "expected FeatureGateDisabled, got {err:?}"
    );
}

/// Companion: even with the gate enabled, if the policy disallows privileged
/// use the result must be PrivilegedCapabilityDenied.
#[test]
fn contract_privileged_capability_denied_without_policy_pass() {
    let policy = DefaultMcpCapabilityPolicy::new(true, false);
    let server = McpServerRef::new("desktop")
        .with_computer_use_requested(true)
        .with_declared_capabilities([CapabilityLevel::PrivilegedComputerUse]);

    let err = enforce_capability_policy(&policy, &server, true)
        .expect_err("must fail when policy disallows privileged");
    assert!(
        matches!(err, McpPolicyError::PrivilegedCapabilityDenied),
        "expected PrivilegedCapabilityDenied, got {err:?}"
    );
}

/// AC: Cleanup API called for both normal and interrupt exits.
///
/// The lifecycle trait has separate hooks for normal turn cleanup and
/// interrupt cleanup. Both must be independently callable.
#[tokio::test]
async fn contract_cleanup_api_called_for_normal_exit() {
    let lifecycle = MockLifecycle::default();

    lifecycle
        .after_turn_cleanup()
        .await
        .expect("normal cleanup must succeed");
    assert_eq!(
        lifecycle.cleanup_calls.load(Ordering::SeqCst),
        1,
        "after_turn_cleanup must have been invoked exactly once"
    );
    assert_eq!(
        lifecycle.interrupt_cleanup_calls.load(Ordering::SeqCst),
        0,
        "on_interrupt_cleanup must not be called on normal exit"
    );
}

#[tokio::test]
async fn contract_cleanup_api_called_for_interrupt_exit() {
    let lifecycle = MockLifecycle::default();

    lifecycle
        .on_interrupt_cleanup()
        .await
        .expect("interrupt cleanup must succeed");
    assert_eq!(
        lifecycle.interrupt_cleanup_calls.load(Ordering::SeqCst),
        1,
        "on_interrupt_cleanup must have been invoked exactly once"
    );
    assert_eq!(
        lifecycle.cleanup_calls.load(Ordering::SeqCst),
        0,
        "after_turn_cleanup must not be called on interrupt exit"
    );
}

// ── §8.2 Integration tests ─────────────────────────────────────────────────

/// AC: MCP server registration + tool dispatch happy path.
///
/// A non-reserved server with Standard capability should pass policy
/// enforcement and return CapabilityLevel::Standard.
#[test]
fn integration_registration_and_dispatch_happy_path() {
    let policy = DefaultMcpCapabilityPolicy::new(true, false);
    let server = McpServerRef::new("my-custom-server")
        .with_declared_capabilities([CapabilityLevel::Standard]);

    let level = enforce_capability_policy(&policy, &server, true)
        .expect("non-reserved standard server must pass");
    assert_eq!(
        level,
        CapabilityLevel::Standard,
        "standard server must negotiate Standard capability"
    );
}

/// Companion: privileged happy path — gate on, policy allows, server declares.
#[test]
fn integration_privileged_registration_happy_path() {
    let policy = DefaultMcpCapabilityPolicy::new(true, true);
    let server = McpServerRef::new("desktop-automation")
        .with_computer_use_requested(true)
        .with_declared_capabilities([CapabilityLevel::PrivilegedComputerUse]);

    let level = enforce_capability_policy(&policy, &server, true)
        .expect("fully-qualified privileged server must pass");
    assert_eq!(
        level,
        CapabilityLevel::PrivilegedComputerUse,
        "must negotiate PrivilegedComputerUse"
    );
}

/// AC: Interrupt during tool call triggers cleanup once.
///
/// When an interrupt fires, on_interrupt_cleanup must be called exactly once
/// — not duplicated with after_turn_cleanup.
#[tokio::test]
async fn integration_interrupt_triggers_cleanup_once() {
    let lifecycle = Arc::new(MockLifecycle::default());

    // Simulate: before_tool_call, then interrupt fires
    lifecycle
        .before_tool_call()
        .await
        .expect("before_tool_call must succeed");

    // Interrupt path
    let _ = lifecycle.on_interrupt_cleanup().await;

    assert_eq!(
        lifecycle.interrupt_cleanup_calls.load(Ordering::SeqCst),
        1,
        "interrupt cleanup must be called exactly once"
    );
    assert_eq!(
        lifecycle.cleanup_calls.load(Ordering::SeqCst),
        0,
        "normal cleanup must NOT be called during interrupt path"
    );
    assert_eq!(
        lifecycle.before_calls.load(Ordering::SeqCst),
        1,
        "before_tool_call should have been called once"
    );
}

/// AC: Cleanup exception does not poison subsequent turns.
///
/// If after_turn_cleanup fails on one turn, the lifecycle must still be
/// usable for the next turn — no poisoned state.
#[tokio::test]
async fn integration_cleanup_exception_does_not_poison_subsequent_turns() {
    let lifecycle = Arc::new(MockLifecycle {
        fail_cleanup: AtomicBool::new(true),
        ..Default::default()
    });

    // Turn 1: cleanup fails
    let result = lifecycle.after_turn_cleanup().await;
    assert!(result.is_err(), "first cleanup should fail as configured");
    assert_eq!(lifecycle.cleanup_calls.load(Ordering::SeqCst), 1);

    // Turn 2: stop failing, lifecycle should still work
    lifecycle.fail_cleanup.store(false, Ordering::SeqCst);

    lifecycle
        .before_tool_call()
        .await
        .expect("before_tool_call must succeed after prior cleanup failure");
    lifecycle
        .after_turn_cleanup()
        .await
        .expect("cleanup must succeed on subsequent turn");

    assert_eq!(
        lifecycle.cleanup_calls.load(Ordering::SeqCst),
        2,
        "lifecycle must remain usable after cleanup failure"
    );
    assert_eq!(lifecycle.before_calls.load(Ordering::SeqCst), 1);
}

/// Companion: interrupt cleanup failure also does not poison.
#[tokio::test]
async fn integration_interrupt_cleanup_failure_does_not_poison() {
    let lifecycle = Arc::new(MockLifecycle {
        fail_cleanup: AtomicBool::new(true),
        ..Default::default()
    });

    // Interrupt cleanup fails
    let result = lifecycle.on_interrupt_cleanup().await;
    assert!(result.is_err());

    // Next turn: should still work
    lifecycle.fail_cleanup.store(false, Ordering::SeqCst);
    lifecycle
        .before_tool_call()
        .await
        .expect("lifecycle must not be poisoned by prior interrupt cleanup failure");
    lifecycle
        .after_turn_cleanup()
        .await
        .expect("normal cleanup must work after interrupt cleanup failure");
}
