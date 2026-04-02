# US-023 MultiAgentOrchestrator Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement a concrete `MultiAgentOrchestrator` with policy enforcement, notification handling, synthesis, idempotent stop semantics, and unit tests covering contract behavior.

**Architecture:** Add an in-memory orchestrator service in `ccode-application` that implements `MultiAgentOrchestrator`. It validates task specs and write-scope conflicts for parallel fan-out, tracks task lifecycle/status and summaries, and synthesizes coordinator output from structured notifications. Use unit tests to drive behavior and error handling.

**Tech Stack:** Rust (`tokio`, `async_trait`, `std::sync`), existing `spec_contracts` types, unit tests with `#[tokio::test]`.

---

### Task 1: Define failing tests for orchestration policy and lifecycle

**Files:**
- Create: `crates/application/src/multi_agent_orchestrator_tests.rs`
- Modify: `crates/application/src/lib.rs`

1. Add tests for overlapping write-scope rejection in `spawn_parallel`.
2. Add tests for synthesis aggregation behavior.
3. Add tests for idempotent stop semantics.
4. Add tests for all `OrchestrationError` variants through behavior.
5. Run targeted tests and confirm failures.

### Task 2: Implement orchestrator service minimally to satisfy tests

**Files:**
- Create: `crates/application/src/multi_agent_orchestrator_service.rs`
- Modify: `crates/application/src/lib.rs`

1. Implement `ApplicationMultiAgentOrchestrator` with in-memory task state.
2. Enforce task spec validity, self-contained prompts with ownership boundaries, and non-overlapping write scopes.
3. Parse/validate notifications and record status/summary.
4. Implement synthesis logic with deterministic aggregation.
5. Implement idempotent stop for terminal states and missing-task handling.

### Task 3: Verify and finalize

**Files:**
- Verify only

1. Run `cargo fmt --check`.
2. Run `cargo clippy --workspace -- -D warnings`.
3. Run `cargo test --workspace`.
4. Commit with `feat: US-023 - MultiAgentOrchestrator implementation`.
