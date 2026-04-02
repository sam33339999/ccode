# US-018 Remote Runtime RemoteSessionService Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement bridge-mode remote session contracts with a strict state machine and stable error mapping between runtime and application service layers.

**Architecture:** Keep transport/auth concerns in `ccode-remote-runtime` (`CcrClientError`, client trait, shared remote session models) and policy/error translation in `ccode-application` via a concrete `ApplicationRemoteSessionService`. Enforce session transition invariants through `RemoteSessionState` helper methods and cover idempotent archive semantics at service and transition levels.

**Tech Stack:** Rust 2024, async-trait, thiserror, workspace crates (`remote-runtime`, `application`).

---

### Task 1: Add failing state-machine and contract tests in remote-runtime

**Files:**
- Create: `crates/remote-runtime/src/contracts_tests.rs`
- Modify: `crates/remote-runtime/src/lib.rs`

1. Write tests for all allowed transitions.
2. Write tests for key forbidden transitions including `Archived -> Running` and `Expired -> Running`.
3. Write tests for terminal-state no-outgoing behavior.
4. Run `cargo test -p ccode-remote-runtime` and confirm failure.

### Task 2: Implement remote-runtime contract types and state machine

**Files:**
- Modify: `crates/remote-runtime/src/contracts.rs`

1. Define `RemoteSessionState` with required 7 states.
2. Add `can_transition_to` and `transition_to` enforcing allowed/forbidden transitions.
3. Add `RemoteSessionError` variants per contract.
4. Add request/summary/policy structs and `RemoteSessionService` trait signatures.
5. Keep runtime-level `CcrClientError` typed transport/auth only.

### Task 3: Add failing application service tests for error translation and archive idempotency

**Files:**
- Create: `crates/application/src/remote_session_service_tests.rs`
- Modify: `crates/application/src/lib.rs`

1. Write tests covering runtime-to-service error mapping for each variant.
2. Write test proving archive treats 409 equivalent (`AlreadyArchived`) as success.
3. Write test for shutdown best-effort archive timeout behavior.
4. Run `cargo test -p ccode-application` and confirm failure.

### Task 4: Implement application RemoteSessionService adapter

**Files:**
- Create: `crates/application/src/remote_session_service.rs`
- Modify: `crates/application/src/spec_contracts.rs`
- Modify: `crates/application/Cargo.toml`
- Modify: `Cargo.toml`

1. Add/align service contract in `spec_contracts` (including missing error variants).
2. Implement `ApplicationRemoteSessionService<C: CcrClient>` using runtime client.
3. Translate runtime errors into stable `RemoteSessionError` variants.
4. Implement best-effort archive helper under timeout budget using `tokio::time::timeout`.

### Task 5: Verify workspace and commit

**Files:**
- No code changes expected

1. Run `cargo fmt --check`.
2. Run `cargo clippy --workspace -- -D warnings`.
3. Run `cargo test --workspace`.
4. Commit with `feat: US-018 - Remote runtime crate — RemoteSessionService`.
