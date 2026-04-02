# Rust Rewrite Spec Polishing (10 Iterations, No Implementation)

## 0. Scope and Constraint

This document defines **10 iterative spec refinements** for the Rust rewrite architecture.  
Constraint: stay close to the current codebase behavior and boundaries (no speculative redesign detached from existing modules).

Evidence anchors (current code):

1. Assistant/mode routing: `main.tsx`, `utils/processUserInput/*`, `commands.ts`
2. Remote control lifecycle: `bridge/*`, `entrypoints/cli.tsx`
3. MCP and computer-use: `services/mcp/*`, `utils/computerUse/*`, `query.ts`
4. Team memory sync: `services/teamMemorySync/*`, `memdir/*`, `setup.ts`
5. Triggers/cron/remote trigger: `tools/ScheduleCronTool/*`, `tools/RemoteTriggerTool/*`, `utils/cron*`
6. Ultraplan flow: `commands/ultraplan.tsx`, `utils/planModeV2.ts`
7. Coordinator mode: `coordinator/*`, `utils/systemPrompt.ts`, `tools.ts`

---

## Iteration 1: Baseline Contract Freeze

### Spec goals

1. Freeze crate boundaries from `README.md`.
2. Freeze codename mapping from `codename-mapping.md`.
3. Define a parity target list (`P0`, `P1` command and tool paths).

### Acceptance tests (spec-level)

1. Document check: all 7 codenames map to explicit Rust crates and owners.
2. Dependency direction check rules are written and non-ambiguous.
3. Every mapped codename has at least one entrypoint + one side-effect sink documented.

---

## Iteration 2: BRIDGE_MODE Policy Precision

### Spec goals

1. Refine `bridge-mode-contract.md` into testable policy outcomes.
2. Pin fail-close rules for entitlement/token/org checks.
3. Pin archive idempotency behavior.

### Acceptance tests (spec-level)

1. Matrix includes outcomes for: no token, no org UUID, gate off, timeout, 409 archive.
2. Session state transition table forbids `Archived -> Running` and `Expired -> Running`.
3. Resume path behavior is specified for not-found vs expired vs archived.

---

## Iteration 3: AGENT_TRIGGERS and AGENT_TRIGGERS_REMOTE Split

### Spec goals

1. Separate local scheduler policy from remote trigger dispatch policy.
2. Define ownership model (agent/team lead/session scope) consistent with current behavior.
3. Define durable vs session-only semantics as first-class contract.

### Acceptance tests (spec-level)

1. Durable tasks require explicit opt-in and persistence path contract (`.claude/scheduled_tasks.json` equivalent policy).
2. Teammate ownership restrictions are specified as errors, not implicit behavior.
3. Remote trigger requires org/auth validation before dispatch.

---

## Iteration 4: CHICAGO_MCP Capability Boundary

### Spec goals

1. Split MCP core transport contracts from computer-use privileged adapter.
2. Freeze reserved server-name policy and built-in default-disabled behavior.
3. Specify turn-end cleanup requirements for computer-use lock/unhide.

### Acceptance tests (spec-level)

1. Contract has explicit rejection cases for reserved names.
2. Cleanup behavior is mandatory on normal end and interrupt paths.
3. Privileged adapter cannot be activated without both feature gate and runtime policy pass.

---

## Iteration 5: TEAMMEM Safety and Conflict Model

### Spec goals

1. Formalize pull/push/sync conflict semantics (412, ETag, retry budget).
2. Formalize path validation and secret scan as mandatory pre-upload/write checks.
3. Define max-entry and max-size policies as typed outcomes.

### Acceptance tests (spec-level)

1. Secret-detected files are excluded with auditable metadata, never raw secret output.
2. Path traversal/symlink escape cases are rejected by contract.
3. Conflict retries have explicit cap and final failure classification.

---

## Iteration 6: ULTRAPLAN Lifecycle Hardening

### Spec goals

1. Specify launch/poll/approve/stop/archive as one state machine.
2. Define orphan session prevention guarantees on error branches.
3. Pin keyword-trigger conditions to avoid recursive self-trigger behavior.

### Acceptance tests (spec-level)

1. Launch failure before session creation yields no archive call.
2. Failure after session creation requires archive attempt + state reset.
3. Concurrent launch attempts produce deterministic "already launching/polling" outcome.

---

## Iteration 7: KAIROS Scope Decomposition

### Spec goals

1. Decompose KAIROS into activation, prompt composition, routing, telemetry facets.
2. Define contract for tool visibility and brief mode interaction.
3. Freeze analytics field semantics (`kairosActive`-equivalent behavior).

### Acceptance tests (spec-level)

1. Prompt construction precedence is explicitly ordered and conflict-resolved.
2. Slash routing behavior in assistant mode is deterministic.
3. Telemetry field emission conditions are documented without leaking sensitive payload.

---

## Iteration 8: COORDINATOR_MODE Consistency

### Spec goals

1. Define precedence between env mode and resumed-session mode.
2. Define coordinator tool-allowlist policy as contract input/output.
3. Specify mode-switch audit events.

### Acceptance tests (spec-level)

1. Resume of session with opposite mode triggers deterministic switch policy.
2. Tool visibility under coordinator mode is testable and not UI-only.
3. Mode-switch event fields are stable and versioned.

---

## Iteration 9: Cross-Codename Integration Contracts

### Spec goals

1. Define interactions between high-risk paths:
Bridge x Ultraplan, MCP x Tool runtime, TeamMem x KAIROS memory prompts.
2. Define unified error envelope and correlation IDs across crates.
3. Define end-to-end observability schema for session/tool/trigger flows.

### Acceptance tests (spec-level)

1. A single correlation ID strategy is documented across remote and local actions.
2. Error taxonomy mapping table exists (`runtime error -> service error -> user-visible class`).
3. Cross-feature race scenarios are listed with expected outcomes.

---

## Iteration 10: Release Readiness Spec Lock

### Spec goals

1. Merge iterations 1-9 into a locked release-candidate spec.
2. Add waiver process (owner, expiry, risk, mitigation).
3. Freeze acceptance gate thresholds and sign-off workflow.

### Acceptance tests (spec-level)

1. All mandatory spec artifacts exist and cross-reference each other.
2. Every `P0` path has explicit acceptance criteria and test type.
3. Final sign-off checklist aligns with `acceptance-spec.md`.

---

## Unified Acceptance Matrix Template (For Each Codename)

Use this template when writing each codename's final contract:

1. **Entry conditions**
Feature gate, auth/entitlement, config constraints.
2. **State model**
Valid states, transitions, terminal states.
3. **Side effects**
Network endpoints, local writes, cleanup duties.
4. **Error classes**
Policy errors, transport errors, data/validation errors.
5. **Observability**
Events, metrics, correlation ID requirements.
6. **Security controls**
Permission checks, secret/path validation, deny-by-default points.
7. **Acceptance tests**
Contract tests, integration tests, CLI behavior tests.

---

## Immediate Next Spec Docs (No Code Yet)

Remaining docs were planned here; all items below are now completed in this session:

1. `docs/rust-rewrite/kairos-contract.md`
2. `docs/rust-rewrite/coordinator-mode-contract.md`
3. `docs/rust-rewrite/spec-lock-v1.md`

This preserves your requirement: spec-first, acceptance-first, no implementation.

## Completion Status (Current Session)

Completed in this session:

1. `docs/rust-rewrite/agent-triggers-contract.md`
2. `docs/rust-rewrite/chicago-mcp-contract.md`
3. `docs/rust-rewrite/teammem-contract.md`
4. `docs/rust-rewrite/ultraplan-contract.md`
5. `docs/rust-rewrite/kairos-contract.md`
6. `docs/rust-rewrite/coordinator-mode-contract.md`
7. `docs/rust-rewrite/integration-contracts.md`
8. `docs/rust-rewrite/spec-lock-v1.md`
