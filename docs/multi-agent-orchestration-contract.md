# Multi-Agent Orchestration Contract Spec (Rust)

## 1. Objective

Define a first-class Rust contract for orchestrating multiple agents that can:

1. Spawn workers in parallel for independent tasks.
2. Receive worker results as structured task notifications.
3. Synthesize results into a single coordinator-level summary.

This is required capability, not optional behavior.

## 2. Evidence from Current Code

1. `tools.ts:3` imports `AgentTool`.
2. `tools.ts:12` imports `TaskStopTool`.
3. `tools.ts:288-295` coordinator mode allows `AgentTool` + `TaskStopTool`.
4. `coordinator/coordinatorMode.ts:140` coordinator must report launched agents and wait for results.
5. `coordinator/coordinatorMode.ts:144` worker results arrive as `<task-notification>`.
6. `coordinator/coordinatorMode.ts:213` explicit parallel-launch guidance.
7. `cli/print.ts:1939-2015` `task-notification` mode handling in main loop.
8. `main.tsx:3035` `teamContext` initialization supports teammate spawning.
9. `main.tsx:3857` hidden `--teammate-mode` supports `auto|tmux|in-process`.

## 3. Rust Boundary Mapping

1. `crates/api-types`
Task notification schema, worker task IDs, orchestration state enums.

2. `crates/app-services`
Orchestration policy: fan-out eligibility, blocking vs sidecar tasks, synthesis requirements.

3. `crates/tool-runtime`
Tool surface integration (`AgentTool`, `TaskStopTool`, message routing hooks).

4. `crates/remote-runtime`
Optional remote worker lifecycle (for remote agents/tasks).

5. `crates/state-store`
Task metadata/transcript pointers and orchestration checkpoints.

6. `crates/config`
Mode and limits (`max_parallel_agents`, teammate mode defaults, guardrails).

7. `bins/app-cli` / `crates/ui-tui`
Display status only; no scheduling policy decisions.

## 4. Core Contracts (Rust Sketch)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchestrationMode {
    Disabled,
    Coordinator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskCriticality {
    Blocking,
    Sidecar,
}

#[derive(Debug, Clone)]
pub struct WorkerTaskSpec {
    pub task_id: String,
    pub title: String,
    pub prompt: String,
    pub criticality: TaskCriticality,
    pub owner_scope: String, // e.g. file/module boundary
}

#[derive(Debug, Clone)]
pub struct WorkerResultNotification {
    pub task_id: String,
    pub status: WorkerStatus,
    pub summary: String,
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[async_trait::async_trait]
pub trait MultiAgentOrchestrator: Send + Sync {
    async fn spawn_parallel(
        &self,
        tasks: Vec<WorkerTaskSpec>,
    ) -> Result<Vec<String>, OrchestrationError>; // returns worker ids

    async fn handle_notification(
        &self,
        notification: WorkerResultNotification,
    ) -> Result<(), OrchestrationError>;

    async fn synthesize_summary(
        &self,
        task_ids: &[String],
    ) -> Result<CoordinatorSummary, OrchestrationError>;

    async fn stop_task(&self, task_id: &str) -> Result<(), OrchestrationError>;
}

#[derive(Debug, Clone)]
pub struct CoordinatorSummary {
    pub completed: usize,
    pub failed: usize,
    pub key_findings: Vec<String>,
    pub next_actions: Vec<String>,
}
```

## 5. Policy Requirements

1. Independent tasks must be launched in parallel when no shared write scope exists.
2. Blocking critical-path work should not be delegated if local progress depends on it immediately.
3. Coordinator must produce synthesized summary from notifications; no raw pass-through only.
4. Worker prompts must be self-contained and include ownership boundaries.
5. Task stop requests must be idempotent and produce deterministic task terminal status.

## 6. Error Taxonomy

```rust
pub enum OrchestrationError {
    DisabledByMode,
    InvalidTaskSpec,
    PolicyViolation,     // e.g. conflicting write scopes in parallel group
    NotificationMalformed,
    TaskNotFound,
    SpawnFailed(String),
    SynthesisFailed(String),
}
```

## 7. Constants Classification

1. `api-types`: orchestration state names, notification type keys.
2. `config`: max parallel count, timeout defaults, teammate mode defaults.
3. `core-domain`: invariants (e.g. terminal-state transition rules).
4. `tool-runtime`: tool-name constants and internal routing keys.
5. `ui-tui`: display labels for task status and coordinator summaries.

## 8. Acceptance Matrix

### 8.1 Contract tests

1. Parallel fan-out rejects overlapping write scopes when policy forbids them.
2. Notification parsing enforces structured payload fields (`task_id`, `status`, `summary`).
3. Synthesis requires completed/failed accounting and deterministic summary structure.

### 8.2 Integration tests

1. Spawn 3 independent tasks in parallel and receive 3 notifications.
2. Mixed result statuses (completed/failed/cancelled) are aggregated correctly.
3. `stop_task` on already-terminal task is idempotent and non-fatal.

### 8.3 CLI behavior tests

1. Coordinator mode exposes multi-agent controls and task status updates.
2. Non-coordinator mode blocks orchestration calls with explicit policy error.
3. Final user-visible summary includes launched tasks, findings, and next actions.

## 9. Relationship to Existing Contracts

1. Extends `coordinator-mode-contract.md` with execution semantics.
2. Depends on `integration-contracts.md` shared error envelope and correlation IDs.
3. Must be listed in `spec-lock-v1.md` before implementation phase begins.
