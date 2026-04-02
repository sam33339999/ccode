# BRIDGE_MODE / CCR Contract Spec (Rust)

## 1. Objective

Define a production-grade Rust contract for Remote Control (BRIDGE_MODE / CCR) that preserves current behavior while enforcing clearer boundaries.

Primary evidence in current code:

1. Entitlement/gate logic: `bridge/bridgeEnabled.ts`
2. Session lifecycle API: `bridge/createSession.ts`
3. Session ID compatibility shim: `bridge/sessionIdCompat.ts`
4. Runtime protocol types and dependencies: `bridge/types.ts`
5. Command entry path and bridge wiring: `entrypoints/cli.tsx:112`
6. Session archive and shutdown behavior: `bridge/createSession.ts:263`

## 2. Rust Ownership Boundaries

### Crate mapping

1. `crates/api-types`
Purpose: shared wire/domain contracts for remote sessions.

2. `crates/app-services`
Purpose: policy and orchestration (gate behavior, retry policy, archive guarantees, resume semantics).

3. `crates/remote-runtime`
Purpose: concrete CCR client implementation (HTTP transport, auth headers, compat session ID translation).

4. `crates/config`
Purpose: feature gate keys, default timeouts, environment variable names.

5. `bins/app-cli`
Purpose: user-facing command entry and message rendering only.

### Architectural rule

`app-cli -> app-services -> remote-runtime -> platform`

`app-services` owns policy decisions. `remote-runtime` must not decide fail-open/fail-close behavior.

## 3. Proposed Contracts (Rust)

### 3.1 API types (`crates/api-types/src/remote_session.rs`)

```rust
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteSessionId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RemoteSessionState {
    Pending,
    Running,
    Idle,
    RequiresAction,
    Archived,
    Expired,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRemoteSessionRequest {
    pub environment_id: EnvironmentId,
    pub title: Option<String>,
    pub permission_mode: Option<String>,
    pub events: Vec<SessionEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub event_type: String,
    pub payload_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSessionSummary {
    pub session_id: RemoteSessionId,
    pub title: Option<String>,
    pub environment_id: Option<EnvironmentId>,
    pub state: RemoteSessionState,
}

#[derive(Debug, Clone, Copy)]
pub struct ArchivePolicy {
    pub timeout: Duration,
    pub idempotent: bool,
}
```

### 3.2 Service trait (`crates/app-services/src/remote_session_service.rs`)

```rust
use crate::errors::RemoteSessionError;
use api_types::remote_session::*;

#[async_trait::async_trait]
pub trait RemoteSessionService: Send + Sync {
    async fn create_session(
        &self,
        req: CreateRemoteSessionRequest,
    ) -> Result<RemoteSessionSummary, RemoteSessionError>;

    async fn fetch_session(
        &self,
        session_id: &RemoteSessionId,
    ) -> Result<RemoteSessionSummary, RemoteSessionError>;

    async fn archive_session(
        &self,
        session_id: &RemoteSessionId,
        policy: ArchivePolicy,
    ) -> Result<(), RemoteSessionError>;

    async fn update_title(
        &self,
        session_id: &RemoteSessionId,
        title: &str,
    ) -> Result<(), RemoteSessionError>;

    async fn reconcile_resume(
        &self,
        session_id: &RemoteSessionId,
    ) -> Result<RemoteSessionSummary, RemoteSessionError>;
}
```

### 3.3 Runtime client trait (`crates/remote-runtime/src/ccr_client.rs`)

```rust
use api_types::remote_session::*;
use crate::error::CcrClientError;

#[async_trait::async_trait]
pub trait CcrClient: Send + Sync {
    async fn create(&self, req: CreateRemoteSessionRequest)
        -> Result<RemoteSessionSummary, CcrClientError>;
    async fn get(&self, session_id: &RemoteSessionId)
        -> Result<RemoteSessionSummary, CcrClientError>;
    async fn archive(&self, session_id: &RemoteSessionId)
        -> Result<ArchiveResult, CcrClientError>;
    async fn patch_title(&self, session_id: &RemoteSessionId, title: &str)
        -> Result<(), CcrClientError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveResult {
    Archived,
    AlreadyArchived, // maps HTTP 409 as success-equivalent
}
```

## 4. State Machine and Invariants

## 4.1 Session state transitions

Allowed:

1. `Pending -> Running | Idle | RequiresAction | Failed`
2. `Running -> Idle | RequiresAction | Archived | Failed | Expired`
3. `Idle -> Running | Archived | Expired`
4. `RequiresAction -> Running | Archived | Expired`
5. `Archived` and `Expired` are terminal.

Forbidden:

1. `Archived -> Running` (must create/resume a new effective session context).
2. `Expired -> Running` (requires explicit recreate/reconnect flow).

## 4.2 Archive semantics

1. Archive is idempotent (`409` is success-equivalent).
2. Missing token/org context is a policy error in `app-services`; runtime should return typed transport/auth errors only.
3. Shutdown path can degrade to best-effort archive, but command path must report deterministic result.

## 5. Error Taxonomy

## 5.1 Service-level errors (`app-services`)

```rust
#[derive(thiserror::Error, Debug)]
pub enum RemoteSessionError {
    #[error("remote control is disabled by build or gate")]
    DisabledByPolicy,
    #[error("missing entitlement or unsupported login profile")]
    EntitlementDenied,
    #[error("invalid session state transition")]
    InvalidStateTransition,
    #[error("session not found")]
    NotFound,
    #[error("session expired")]
    Expired,
    #[error("auth unavailable")]
    AuthUnavailable,
    #[error("upstream transport error: {0}")]
    Upstream(String),
}
```

## 5.2 Runtime-level errors (`remote-runtime`)

```rust
#[derive(thiserror::Error, Debug)]
pub enum CcrClientError {
    #[error("http error: {0}")]
    Http(String),
    #[error("timeout")]
    Timeout,
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("invalid response payload")]
    InvalidPayload,
}
```

Policy: runtime errors are translated in `app-services` into stable `RemoteSessionError`.

## 6. Compatibility Rules (session_ vs cse_)

Based on `bridge/sessionIdCompat.ts` behavior:

1. Keep ID compatibility translation in `remote-runtime` only.
2. `api-types` stores canonical `RemoteSessionId` as opaque string.
3. `app-services` never manipulates tag prefixes directly.

Rationale:

1. Prefix translation is transport compatibility logic, not domain logic.
2. Prevents spread of wire-format assumptions across crates.

## 7. Constants Classification (BRIDGE_MODE)

Place constants by semantics:

1. `api-types`: state labels and event names (`RemoteSessionState`, archive result categories).
2. `config`: gate keys (`tengu_ccr_bridge`), timeout defaults, env var names.
3. `core-domain`: state transition invariants (allowed transitions).
4. `remote-runtime`: endpoint path fragments and header wiring details.
5. `ui-tui`: user-facing copy (login guidance, disconnected messages).

## 8. Acceptance Criteria for BRIDGE_MODE

## 8.1 Contract tests

1. `archive(409)` maps to success-equivalent result.
2. `session_*` and `cse_*` IDs round-trip through compatibility adapter correctly.
3. Missing token/org produces `AuthUnavailable` or `EntitlementDenied` (never panic).

## 8.2 Integration tests

1. Create -> fetch -> archive happy path passes.
2. Resume with stale/expired session returns deterministic error classification.
3. Shutdown path performs best-effort archive under timeout budget.

## 8.3 CLI behavior tests

1. Disabled gate path prints actionable message.
2. Successful remote-control path yields session URL and lifecycle updates.
3. Archive failure surfaces non-fatal warning and exit remains graceful.

## 9. Migration Sequence (Bridge Slice)

1. Create `api-types::remote_session` models and serde contracts.
2. Define `RemoteSessionService` trait in `app-services`.
3. Implement `CcrClient` + HTTP adapter in `remote-runtime`.
4. Add compatibility adapter for session ID tags in `remote-runtime`.
5. Wire `bins/app-cli` command flow to `RemoteSessionService`.
6. Add contract/integration tests and enforce in CI.

Exit condition:

1. Bridge vertical slice works end-to-end in Rust with test-backed parity for create/fetch/archive/resume behaviors.
