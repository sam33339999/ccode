# Rust Rewrite Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Rewrite this TypeScript/Bun CLI project into a Rust-first codebase using `cargo workspace` with explicit `lib + bin` boundaries, optimized for enterprise maintainability and long-term evolution.

**Architecture:** Use a layered workspace where domain logic, orchestration, integration adapters, and UI/CLI are separated into focused crates. Keep contracts and shared protocol types stable and centrally governed. Enforce one-way dependencies (`domain -> services -> adapters -> bins`) to prevent architectural erosion.

**Tech Stack:** Rust stable, Cargo workspace, `clap`, `tokio`, `serde`, `thiserror`, `tracing`, `config`, `sqlx` or `sea-orm` (if needed), `nextest`, `clippy`, `rustfmt`.

---

## 1. Decision Summary

This rewrite is **full migration to Rust**, but **not flat migration**.  
We will not copy the current folder tree 1:1 into one giant crate. Instead, we split by responsibility and coupling:

1. Stable contracts and domain policies must live in low-level crates.
2. Operational concerns (MCP, bridge, remote, plugins, tool execution) must be in adapter/runtime crates.
3. The CLI binary remains a thin composition layer.

This design is chosen for enterprise goals:

1. Lower blast radius for changes.
2. Clear ownership boundaries by crate.
3. Faster parallel development across teams.
4. Better auditability, security review, and dependency governance.

## 2. Target Cargo Workspace Layout

```txt
/
├─ Cargo.toml
├─ crates/
│  ├─ api-types/               # shared contracts, protocol models, event names
│  ├─ llm-compat/              # provider-agnostic LLM adapter (Anthropic ↔ Canonical ↔ OpenAI)
│  ├─ config/                  # env parsing, defaults, feature/config policies
│  ├─ core-domain/             # pure business logic and policies
│  ├─ app-services/            # use-case orchestration, agent loop, transactional flows
│  ├─ state-store/             # local/session/team memory and persistence adapters
│  ├─ tool-runtime/            # tool lifecycle, orchestration, streaming execution
│  ├─ mcp-runtime/             # MCP protocol handling and approvals
│  ├─ remote-runtime/          # remote sessions/websocket/bridge behavior
│  ├─ plugin-runtime/          # plugin loading, capability model, registries
│  ├─ ui-tui/                  # terminal rendering and interaction
│  ├─ platform/                # fs/process/network abstractions
│  └─ test-support/            # integration fixtures and test harness utilities
├─ bins/
│  ├─ app-cli/                 # main CLI entrypoint
│  └─ app-daemon/              # optional background service
├─ xtask/                      # release/build/codegen/dev automation
└─ docs/
```

## 3. Why This Cut (Reflection)

### 3.1 Why not a single mega crate

1. Build time and test time degrade rapidly.
2. Ownership becomes unclear.
3. Small changes force full regression cycles.
4. Security review becomes expensive because all capabilities co-exist in one compilation unit.

### 3.2 Why not purely feature-folder crates

Feature slices are useful, but this system has strong runtime concerns (tool orchestration, remote sessions, plugin lifecycle, MCP approvals). Layer-driven crates make production control points explicit and easier to secure.

### 3.3 Why `lib + bin` instead of only `lib`

Only `bin` crates should own process lifecycle concerns (signals, startup order, bootstrapping).  
All reusable logic stays in `lib` crates for testability and composability.

## 4. Dependency Rules (Enforced)

Allowed direction:

1. `api-types` / `config` / `core-domain` -> no dependency on runtime adapters or UI.
2. `llm-compat` -> depends on `api-types` only. Converts between Canonical types and provider wire formats.
3. `app-services` -> depends on domain + contracts + `llm-compat` (via trait, not concrete client).
4. `*-runtime` crates -> depend on services/contracts, never the opposite.
5. `ui-tui` -> depends on services/contracts, not runtime internals.
6. `bins/*` -> wire dependencies only; no business logic.

Forbidden:

1. `core-domain` importing `tokio`, CLI UI crates, or MCP transport code.
2. `api-types` depending on runtime crates or `llm-compat`.
3. `llm-compat` depending on `app-services` or any runtime crate.
4. Cross-runtime cyclic imports (for example `tool-runtime <-> mcp-runtime`).

### LLM Adapter Layer Rule

`llm-compat` is the **sole boundary** between the agent and LLM provider APIs.
All upstream crates (`app-services`, `tool-runtime`, etc.) operate on Canonical types defined in `api-types`.
No provider-specific request/response types may leak above `llm-compat`.

## 5. Service Boundary Mapping (Current -> Target)

From current root folders to Rust boundaries:

1. `commands/`, `commands.ts` -> `ui-tui` + `app-cli` command registry.
2. `tools/`, `tools.ts`, `services/tools/*` -> `tool-runtime`.
3. `services/mcp*`, MCP approval paths -> `mcp-runtime`.
4. `remote/*`, `bridge/*` -> `remote-runtime`.
5. `plugins/*`, `skills/*` loading lifecycle -> `plugin-runtime`.
6. `state/*`, `memdir/*`, `migrations/*` -> `state-store`.
7. `context/*`, `tasks/*`, `query*` orchestration -> `app-services`.
8. `types/*`, `schemas/*` contracts -> `api-types`.
9. `constants/*`, env defaults -> split by constants policy below.
10. LLM API call paths (Anthropic Messages API, OpenAI Chat Completions API) -> `llm-compat`.

## 6. Constants Classification Policy

Do not create one global `constants` dump crate. Use this policy:

1. **Contract Constants** -> `api-types`
Definition: shared protocol names and stable cross-crate identifiers.  
Examples: event keys, RPC method names, schema versions, tool IDs used across runtimes.

2. **Configurable Defaults** -> `config`
Definition: values that may vary by deploy environment or policy.  
Examples: timeout defaults, retry caps, feature toggles, env var names.

3. **Domain Invariants** -> `core-domain`
Definition: business constraints that are part of core rules and should not be environment-driven.  
Examples: validation bounds, state transition guards.

4. **Runtime-Local Constants** -> owning runtime crate
Definition: constants private to one runtime implementation.  
Examples: MCP adapter backoff profile used only in `mcp-runtime`.

5. **UI Presentation Constants** -> `ui-tui`
Definition: rendering/layout/help text constants used only by TUI.

Decision rule:

1. Used by exactly one crate -> keep local.
2. Used by multiple crates as a contract -> `api-types`.
3. Depends on env/policy -> `config`.
4. Represents business truth, not deployment choice -> `core-domain`.

## 7. Implementation Phases (Full Rewrite)

### Phase A: Foundation

1. Create workspace with all target crates.
2. Add lint/test/security baseline (`fmt`, `clippy`, `nextest`, audit).
3. Add crate-level README + ownership metadata.

Exit criteria:

1. Workspace compiles with empty skeletons.
2. CI executes all checks successfully.

### Phase B: Contract and Core

1. Define canonical contracts in `api-types`.
2. Move core policies to `core-domain`.
3. Create `app-services` orchestration traits and interfaces.

Exit criteria:

1. Domain logic unit tests pass.
2. Cross-crate contracts versioned and documented.

### Phase C: Runtime Engines

1. Implement `tool-runtime`, `mcp-runtime`, `remote-runtime`, `plugin-runtime`.
2. Implement state persistence in `state-store`.
3. Wire observability (`tracing`, structured errors, correlation IDs).

Exit criteria:

1. Integration tests validate major runtime flows.
2. Error taxonomy complete and mapped to operator-facing outputs.

### Phase D: CLI and UX

1. Implement command routing in `app-cli`.
2. Implement terminal UX in `ui-tui`.
3. Add compatibility mode for old command names/aliases.

Exit criteria:

1. CLI parity test suite reaches agreed threshold.
2. Startup and interactive flows meet SLO targets.

### Phase E: Hardening and Cutover

1. Security review and threat model verification.
2. Performance and soak testing.
3. Release packaging and rollback playbook.

Exit criteria:

1. Production readiness checklist complete.
2. TS/Bun path retired after stable release window.

## 8. Enterprise Maintainability Controls

1. **Crate ownership:** each crate has owner team and review gate.
2. **ADR discipline:** every boundary change requires ADR in `docs/rust-rewrite/adr/`.
3. **Public API governance:** forbid exposing internals by default (`pub(crate)` first).
4. **Error model standardization:** one typed error taxonomy across crates.
5. **Observability baseline:** structured logs, trace IDs, key metrics in all runtimes.
6. **Security posture:** dependency policy, secret scanning, permission boundary tests.
7. **Upgrade strategy:** scheduled dependency update windows and compatibility checks.

## 9. Testing and Verification Strategy

Test layers:

1. Unit tests in each crate for pure logic.
2. Contract tests for `api-types` stability.
3. Integration tests per runtime crate.
4. End-to-end CLI smoke tests from `app-cli`.

Quality gates:

1. `cargo fmt --check`
2. `cargo clippy --all-targets --all-features -D warnings`
3. `cargo nextest run --workspace`
4. Optional: `cargo audit`, `cargo deny`

## 10. Risks and Mitigations

1. **Risk:** Hidden coupling in current TS codebase.
Mitigation: create dependency map before moving each subsystem.

2. **Risk:** Boundary drift after initial migration.
Mitigation: enforce dependency rules via workspace-level lint/check scripts.

3. **Risk:** Performance regressions in async paths.
Mitigation: benchmark critical commands before cutover and gate releases.

4. **Risk:** Incomplete CLI parity.
Mitigation: prioritize high-frequency command parity first and track gaps explicitly.

## 11. Execution Backlog (First Wave)

### Task 1: Bootstrap workspace

**Files:**
- Create: `Cargo.toml`
- Create: `crates/*/Cargo.toml`
- Create: `bins/app-cli/Cargo.toml`
- Create: `xtask/Cargo.toml`

Steps:

1. Create workspace members and shared dependency versions.
2. Add baseline lint/test profiles.
3. Confirm full workspace compiles.

### Task 2: Establish boundary contracts

**Files:**
- Create: `crates/api-types/src/lib.rs`
- Create: `crates/core-domain/src/lib.rs`
- Create: `crates/app-services/src/lib.rs`

Steps:

1. Define foundational request/response/event contracts.
2. Define domain trait boundaries.
3. Add compile-time tests for contract compatibility.

### Task 3: Constants governance

**Files:**
- Create: `docs/rust-rewrite/constants-policy.md`
- Create: `crates/config/src/lib.rs`
- Modify: `crates/api-types/src/lib.rs`
- Modify: `crates/core-domain/src/lib.rs`

Steps:

1. Add constants decision matrix and examples.
2. Seed each target crate with representative constants.
3. Add lint/check to prevent accidental constants sprawl.

### Task 4: Runtime vertical slice

**Files:**
- Create: `crates/tool-runtime/src/lib.rs`
- Create: `crates/mcp-runtime/src/lib.rs`
- Create: `bins/app-cli/src/main.rs`

Steps:

1. Implement one end-to-end command path from CLI to runtime and back.
2. Add integration tests for this slice.
3. Use it as template for remaining subsystems.

## 12. Definition of Done

The rewrite is considered complete when:

1. All required CLI workflows are served by Rust binaries.
2. All runtime responsibilities are covered by crate-aligned ownership.
3. Constants classification policy is applied and enforced.
4. CI quality gates pass for workspace.
5. Runbook, ADRs, and operator docs are complete.

## 13. Acceptance Reference

Formal and testable acceptance criteria are defined in:

1. `docs/rust-rewrite/acceptance-spec.md`
2. `docs/rust-rewrite/codename-mapping.md`
3. `docs/rust-rewrite/bridge-mode-contract.md`
4. `docs/rust-rewrite/spec-iterations.md`
5. `docs/rust-rewrite/agent-triggers-contract.md`
6. `docs/rust-rewrite/chicago-mcp-contract.md`
7. `docs/rust-rewrite/teammem-contract.md`
8. `docs/rust-rewrite/ultraplan-contract.md`
9. `docs/rust-rewrite/kairos-contract.md`
10. `docs/rust-rewrite/coordinator-mode-contract.md`
11. `docs/multi-agent-orchestration-contract.md`
12. `docs/integration-contracts.md`
13. `docs/llm-compat-contract.md`
14. `docs/spec-lock-v1.md`
15. `docs/acceptance-playbook.md`
16. `docs/implementation-phases.md`

Release readiness must be evaluated against that file, not by subjective review.
