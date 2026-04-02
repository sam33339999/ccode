# US-019 CcrClient HTTP Transport Layer Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement a concrete HTTP `CcrClient` in `remote-runtime` with auth/org header attachment, retry+timeout policy, and session ID compatibility translation (`session_` ↔ `cse_`) fully isolated from application services.

**Architecture:** Keep `CcrClient` trait and transport compatibility logic in `ccode-remote-runtime`. Add a small compatibility adapter module for ID prefix conversion, a config struct for timeout/retry tuning, and an auth context provider abstraction for automatic headers. Keep `ccode-application` focused on translating runtime transport errors into service-layer `RemoteSessionError`.

**Tech Stack:** Rust 2024, `reqwest`, `tokio`, `serde`, `serde_json`, `thiserror`, workspace crates (`remote-runtime`, `application`).

---

### Task 1: Add failing tests for compatibility adapter and error classification

**Files:**
- Create: `crates/remote-runtime/src/ccr_client_tests.rs`
- Modify: `crates/remote-runtime/src/lib.rs`

1. Add round-trip tests for `session_*` and `cse_*` prefix translation.
2. Add tests that verify each `CcrClientError` classification path.
3. Run `cargo test -p ccode-remote-runtime` and confirm tests fail first.

### Task 2: Implement HTTP CcrClient and compatibility adapter

**Files:**
- Create: `crates/remote-runtime/src/ccr_client.rs`
- Modify: `crates/remote-runtime/src/contracts.rs`
- Modify: `crates/remote-runtime/src/lib.rs`
- Modify: `crates/remote-runtime/Cargo.toml`

1. Add `HttpCcrClient` implementing `CcrClient` create/get/archive/patch_title.
2. Add retry+timeout config type and request execution loop.
3. Auto-attach auth token + org context headers on every request.
4. Translate HTTP/parse/timeouts into `CcrClientError` variants.
5. Keep `session_`/`cse_` conversion private to remote-runtime adapter.

### Task 3: Align application layer error translation contract

**Files:**
- Modify: `crates/application/src/spec_contracts.rs`
- Modify: `crates/application/src/remote_session_service.rs`
- Modify: `crates/application/src/remote_session_service_tests.rs`

1. Keep `RemoteSessionError` separate from `CcrClientError`.
2. Ensure app-services performs only error translation, never prefix translation.
3. Update tests for any changed `CcrClientError` variant mapping.

### Task 4: Verify and commit

**Files:**
- No new files expected

1. Run `cargo fmt --check`.
2. Run `cargo clippy --workspace -- -D warnings`.
3. Run `cargo test --workspace`.
4. Commit with `feat: US-019 - CcrClient HTTP transport layer`.
