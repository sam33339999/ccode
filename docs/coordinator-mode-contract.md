# COORDINATOR_MODE Contract Spec (Rust)

## 1. Objective

Define Rust rewrite contract for coordinator mode with stable mode-state precedence and tool-allowlist behavior.

## 2. Current Code Evidence

1. `coordinator/coordinatorMode.ts:37` (mode enable check)
2. `coordinator/coordinatorMode.ts:66` (env mode switch)
3. `coordinator/coordinatorMode.ts:71` (mode-switch telemetry)
4. `utils/systemPrompt.ts:63` (coordinator prompt branch)
5. `tools.ts:120`, `280-293` (tool filtering by coordinator mode)
6. `cli/print.ts:4918`, `5123` (resume-mode reconciliation)

## 3. Rust Boundary Mapping

1. `api-types`: mode state enums and mode-switch event schema.
2. `app-services`: effective mode resolution and reconciliation policy.
3. `tool-runtime`: mode-driven tool allowlist filtering.
4. `ui-tui`: mode-aware prompt construction and messaging.
5. `config`: env var and gate keys.

## 4. Proposed Contracts

```rust
pub trait ModeCoordinatorService: Send + Sync {
    fn resolve_effective_mode(&self, input: ModeResolutionInput) -> EffectiveMode;
    fn reconcile_on_resume(&self, session_mode: Option<SessionMode>) -> ModeReconcileResult;
}
```

Required policy:

1. Explicit precedence rule: resumed-session mode vs env mode.
2. Tool allowlist output is deterministic and testable.
3. Mode-switch events emit stable metadata.

## 5. Error Taxonomy

```rust
pub enum CoordinatorModeError {
    DisabledByPolicy,
    InvalidModeTransition,
    SessionModeMismatch,
}
```

## 6. Constants Classification

1. `api-types`: mode names and event keys.
2. `config`: env var keys (`CLAUDE_CODE_COORDINATOR_MODE`) and gate values.
3. `core-domain`: mode transition invariants.
4. `ui-tui`: display-only copy.

## 7. Acceptance Matrix

1. Contract tests: precedence and reconcile logic.
2. Integration tests: resume path updates mode consistently.
3. CLI behavior tests: tool visibility and prompt mode branch are correct.

