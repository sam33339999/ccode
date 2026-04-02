# AGENT_TRIGGERS Contract Spec (Rust)

## 1. Objective

Define Rust contracts for local scheduler (`AGENT_TRIGGERS`) and remote trigger dispatch (`AGENT_TRIGGERS_REMOTE`) with explicit ownership, durability, and authorization behavior.

## 2. Evidence from Current Code

1. `tools.ts:29` (`feature('AGENT_TRIGGERS')`) and cron tools registration.
2. `tools.ts:36` (`feature('AGENT_TRIGGERS_REMOTE')`) and remote tool registration.
3. `skills/bundled/index.ts:56` remote scheduling skill registration.
4. `tools/ScheduleCronTool/CronCreateTool.ts` durable/session-only semantics.
5. `tools/ScheduleCronTool/CronDeleteTool.ts` ownership constraints.
6. `utils/cronTasks.ts:1` durable storage at `.claude/scheduled_tasks.json`.
7. `tools/RemoteTriggerTool/RemoteTriggerTool.ts:59` remote gate (`tengu_surreal_dali`).
8. `cli/print.ts:2705` scheduler boot path in print/SDK flows.

## 3. Rust Boundary Mapping

1. `crates/api-types`: trigger models, task state, owner scope enums.
2. `crates/app-services`: scheduling/authorization policy decisions.
3. `crates/tool-runtime`: trigger tools lifecycle and execution orchestration.
4. `crates/state-store`: durable task persistence abstraction.
5. `crates/remote-runtime`: remote trigger API client.
6. `crates/config`: gate keys, retry/timeouts, file path defaults.
7. `bins/app-cli` and `crates/ui-tui`: command UX and text output only.

## 4. Core Contracts (Rust Sketch)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerScope { SessionOnly, Durable }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerOwner { MainAgent, Teammate(String), TeamLead }

#[derive(Debug, Clone)]
pub struct TriggerTask {
    pub id: String,
    pub cron: String,
    pub prompt: String,
    pub scope: TriggerScope,
    pub owner: TriggerOwner,
    pub recurring: bool,
}

#[async_trait::async_trait]
pub trait TriggerSchedulerService {
    async fn create(&self, task: TriggerTask) -> Result<TriggerTask, TriggerError>;
    async fn list(&self) -> Result<Vec<TriggerTask>, TriggerError>;
    async fn delete(&self, id: &str, actor: TriggerOwner) -> Result<(), TriggerError>;
}

#[async_trait::async_trait]
pub trait RemoteTriggerDispatchService {
    async fn dispatch(&self, payload: serde_json::Value) -> Result<String, TriggerError>;
}
```

## 5. Error Taxonomy

1. `GateDisabled`
2. `InvalidCron`
3. `OwnershipViolation`
4. `DurableNotAllowedForTeammate`
5. `Unauthorized`
6. `UpstreamRemoteError`
7. `StorageError`

## 6. Policy Rules

1. Durable tasks require explicit user intent.
2. Teammates cannot create durable tasks.
3. Delete operation checks owner identity.
4. Remote dispatch requires both feature gate and auth/org context.

## 7. Constants Classification

1. `api-types`: trigger state labels and protocol field names.
2. `config`: gate keys (`tengu_kairos_cron`, `tengu_surreal_dali`), path defaults, timeout defaults.
3. `core-domain`: ownership invariants.
4. `state-store`/`tool-runtime`: local internal key names and lock file internals.
5. `ui-tui`: user-facing messages.

## 8. Acceptance Matrix

### Contract tests

1. Durable + teammate returns `DurableNotAllowedForTeammate`.
2. Invalid cron returns `InvalidCron`.
3. Owner mismatch delete returns `OwnershipViolation`.

### Integration tests

1. Session-only task survives runtime tick but not process restart.
2. Durable task reloads from store on restart.
3. Remote dispatch with gate off returns `GateDisabled`.

### CLI behavior tests

1. List shows scope and owner consistently.
2. Delete reports deterministic reason on ownership error.
3. Remote trigger failure prints non-leaky error class.

