# US-004 Session Management CLI Commands Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add CLI commands to list, show, delete, and clear sessions so users can manage conversation history.

**Architecture:** Extend the existing `sessions` CLI subcommand with new actions and keep logic in small helper functions that are unit-testable. Use the existing session repository port for persistence and add a repository bulk-delete operation for `clear` semantics.

**Tech Stack:** Rust, Clap, Tokio, workspace crates (`cli`, `ports`, `session`, `bootstrap`).

---

### Task 1: Add failing CLI behavior tests

**Files:**
- Modify: `crates/cli/src/cmd/sessions.rs`

1. Write unit tests for output formatting and clear confirmation behavior.
2. Add tests that assert show rendering includes all messages.
3. Add tests for clear confirmation parsing (`yes` required).
4. Run focused tests and confirm failure before implementation.

### Task 2: Implement sessions CLI actions

**Files:**
- Modify: `crates/cli/src/cmd/sessions.rs`

1. Add `Show`, `Delete`, `Clear` variants to action enum.
2. Implement handlers using repository calls.
3. Add confirmation prompt flow for `clear`.
4. Format list output with timestamp and message count columns.

### Task 3: Add repository clear support

**Files:**
- Modify: `crates/ports/src/repositories.rs`
- Modify: `crates/session/src/in_memory.rs`
- Modify: `crates/session/src/jsonl.rs`
- Modify: `crates/application/src/queries/sessions_list.rs` (mock trait compliance)

1. Add `clear_all` to `SessionRepository`.
2. Implement in-memory and file-backed behavior.
3. Update mock repos used in tests for trait compatibility.

### Task 4: Verify and commit

**Files:**
- No code changes expected

1. Run `cargo fmt --check`.
2. Run `cargo clippy --workspace -- -D warnings`.
3. Run `cargo test --workspace`.
4. Commit with message `feat: US-004 - Session management CLI commands`.
