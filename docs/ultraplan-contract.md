# ULTRAPLAN Contract Spec (Rust)

## 1. Objective

Define Rust rewrite contracts for ULTRAPLAN launch and remote planning lifecycle with explicit orphan-prevention and concurrency rules.

## 2. Evidence Anchors (Current Code)

1. Command registration gate: `commands.ts:104`
2. Keyword auto-route: `utils/processUserInput/processUserInput.ts:468`
3. Launch flow and teleport call: `commands/ultraplan.tsx:330`
4. Error-path archive cleanup: `commands/ultraplan.tsx:395`
5. Concurrent launch/poll guard: `commands/ultraplan.tsx:264`
6. Plan-mode permission UI integration: `components/permissions/ExitPlanModePermissionRequest/ExitPlanModePermissionRequest.tsx:144`
7. Polling approval flow: `commands/ultraplan.tsx:83`

## 3. Rust Ownership Boundaries

1. `crates/api-types`
Ultraplan state, poll phase, launch/stop result contracts.
2. `crates/app-services`
Ultraplan orchestration and state machine policy.
3. `crates/remote-runtime`
Remote session creation, polling, archive transport.
4. `crates/config`
Timeout defaults, gate keys, model defaults.
5. `bins/app-cli`
Command wiring and user messaging.
6. `crates/ui-tui`
Interactive prompt/permission view state only.

## 4. State Machine

States:

1. `Idle`
2. `Launching`
3. `Polling`
4. `AwaitingInput`
5. `Approved`
6. `Stopping`
7. `Completed`
8. `Failed`

Allowed transitions:

1. `Idle -> Launching`
2. `Launching -> Polling | Failed`
3. `Polling -> AwaitingInput | Approved | Failed | Stopping`
4. `AwaitingInput -> Polling | Stopping`
5. `Approved -> Completed`
6. `Stopping -> Completed | Failed`

Terminal: `Completed`, `Failed`

## 5. Proposed Contracts (Rust)

```rust
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UltraplanPhase {
    Idle,
    Launching,
    Polling,
    AwaitingInput,
    Approved,
    Stopping,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UltraplanSession {
    pub session_id: String,
    pub session_url: String,
    pub phase: UltraplanPhase,
}

#[derive(Debug, Clone, Copy)]
pub struct UltraplanPolicy {
    pub timeout: Duration,
    pub single_active_session: bool,
}
```

```rust
#[async_trait::async_trait]
pub trait UltraplanService: Send + Sync {
    async fn launch(&self, prompt: &str, policy: UltraplanPolicy) -> Result<UltraplanSession, UltraplanError>;
    async fn poll(&self, session_id: &str) -> Result<UltraplanPhase, UltraplanError>;
    async fn stop(&self, session_id: &str) -> Result<(), UltraplanError>;
    async fn archive_orphan(&self, session_id: &str) -> Result<(), UltraplanError>;
}
```

## 6. Policy Rules

1. At most one active ULTRAPLAN session per local session context.
2. Launch attempts while `Launching/Polling` return deterministic concurrency error.
3. If failure happens after remote session creation, archive must be attempted.
4. Stop operation clears local active session marker before returning.
5. Keyword auto-route must respect launch/polling guards.

## 7. Error Taxonomy

```rust
#[derive(thiserror::Error, Debug)]
pub enum UltraplanError {
    #[error("feature disabled by policy")]
    DisabledByPolicy,
    #[error("already active")]
    AlreadyActive,
    #[error("launch failed")]
    LaunchFailed,
    #[error("poll timeout")]
    PollTimeout,
    #[error("approval failed")]
    ApprovalFailed,
    #[error("archive failed")]
    ArchiveFailed,
    #[error("transport error: {0}")]
    Transport(String),
}
```

## 8. Constants Classification

1. `api-types`: phase enums and lifecycle event names.
2. `config`: launch timeout, poll interval, gate keys, model defaults.
3. `core-domain`: transition invariants for ultraplan phase graph.
4. `remote-runtime`: endpoint path and transport retry constants.
5. `ui-tui`: usage/help strings.

## 9. Acceptance Test Matrix

### 9.1 Contract tests

1. Invalid phase transition rejected.
2. Concurrent launch attempt returns `AlreadyActive`.
3. Stop/cleanup from active state transitions deterministically.

### 9.2 Integration tests

1. Launch -> Poll -> Approved -> Completed happy path.
2. Launch success + downstream failure triggers orphan archive attempt.
3. Poll timeout path returns `PollTimeout`.

### 9.3 CLI behavior tests

1. Bare command usage path returns expected guidance.
2. Keyword-trigger while active does not re-launch.
3. Stop command clears local active markers and session URL state.

