/// Agent triggers acceptance tests (US-044)
///
/// Covers all scenarios from agent-triggers-contract.md §8 Acceptance Matrix:
///   §8.1 Contract tests  – DurableNotAllowedForTeammate, InvalidCron, OwnershipViolation
///   §8.2 Integration tests – session-only vs durable persistence, remote gate disabled
///   §8.3 CLI behaviour tests – list consistency, delete error reason, non-leaky remote error
use crate::spec_contracts::{
    RemoteTriggerDispatchService, TriggerError, TriggerOwner, TriggerSchedulerService,
    TriggerScope, TriggerTask,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

// ── helpers ───────────────────────────────────────────────────────────────

fn task(id: &str, scope: TriggerScope, owner: TriggerOwner) -> TriggerTask {
    TriggerTask {
        id: id.to_string(),
        cron: "0 9 * * *".to_string(),
        prompt: "do work".to_string(),
        scope,
        owner,
        durable_intent: scope == TriggerScope::Durable,
    }
}

// ── In-memory scheduler implementing contract rules ──────────────────────

struct InMemoryTriggerScheduler {
    tasks: Mutex<HashMap<String, TriggerTask>>,
    durable_store: Mutex<HashMap<String, TriggerTask>>,
    gate_enabled: bool,
}

impl InMemoryTriggerScheduler {
    fn new(gate_enabled: bool) -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            durable_store: Mutex::new(HashMap::new()),
            gate_enabled,
        }
    }

    /// Simulate a process restart: drop in-memory tasks, reload from durable store.
    fn restart(&self) -> Self {
        let durable = self.durable_store.lock().expect("lock").clone();
        Self {
            tasks: Mutex::new(durable.clone()),
            durable_store: Mutex::new(durable),
            gate_enabled: self.gate_enabled,
        }
    }
}

#[async_trait]
impl TriggerSchedulerService for InMemoryTriggerScheduler {
    async fn create(&self, task: TriggerTask) -> Result<TriggerTask, TriggerError> {
        if !self.gate_enabled {
            return Err(TriggerError::GateDisabled);
        }
        // Contract: invalid cron
        if !task.cron.contains('*') && !task.cron.contains('/') {
            return Err(TriggerError::InvalidCron);
        }
        // Contract: durable + teammate → DurableNotAllowedForTeammate
        if task.scope == TriggerScope::Durable {
            if matches!(task.owner, TriggerOwner::Teammate(_)) {
                return Err(TriggerError::DurableNotAllowedForTeammate);
            }
        }
        // Persist durable tasks to the durable store
        if task.scope == TriggerScope::Durable {
            self.durable_store
                .lock()
                .expect("lock")
                .insert(task.id.clone(), task.clone());
        }
        self.tasks
            .lock()
            .expect("lock")
            .insert(task.id.clone(), task.clone());
        Ok(task)
    }

    async fn list(&self) -> Result<Vec<TriggerTask>, TriggerError> {
        if !self.gate_enabled {
            return Err(TriggerError::GateDisabled);
        }
        let tasks = self.tasks.lock().expect("lock");
        let mut list: Vec<TriggerTask> = tasks.values().cloned().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(list)
    }

    async fn delete(&self, id: &str, actor: TriggerOwner) -> Result<(), TriggerError> {
        if !self.gate_enabled {
            return Err(TriggerError::GateDisabled);
        }
        let tasks = self.tasks.lock().expect("lock");
        let existing = tasks.get(id).ok_or(TriggerError::OwnershipViolation)?;
        // Contract: owner mismatch → OwnershipViolation
        if existing.owner != actor {
            return Err(TriggerError::OwnershipViolation);
        }
        drop(tasks);
        self.tasks.lock().expect("lock").remove(id);
        self.durable_store.lock().expect("lock").remove(id);
        Ok(())
    }
}

// ── Mock remote dispatch service ─────────────────────────────────────────

struct MockRemoteDispatch {
    gate_enabled: bool,
    transport_error: Option<String>,
}

impl MockRemoteDispatch {
    fn enabled() -> Self {
        Self {
            gate_enabled: true,
            transport_error: None,
        }
    }

    fn gate_disabled() -> Self {
        Self {
            gate_enabled: false,
            transport_error: None,
        }
    }

    fn with_transport_error(detail: &str) -> Self {
        Self {
            gate_enabled: true,
            transport_error: Some(detail.to_string()),
        }
    }
}

#[async_trait]
impl RemoteTriggerDispatchService for MockRemoteDispatch {
    async fn dispatch(&self, _payload: Value) -> Result<String, TriggerError> {
        if !self.gate_enabled {
            return Err(TriggerError::GateDisabled);
        }
        if self.transport_error.is_some() {
            return Err(TriggerError::UpstreamRemoteError);
        }
        Ok("remote-run-id-1".to_string())
    }
}

// ── §8.1 Contract tests ─────────────────────────────────────────────────

/// AC: durable + teammate returns DurableNotAllowedForTeammate.
///
/// The contract forbids teammates from creating durable-scoped tasks.
#[tokio::test]
async fn contract_durable_teammate_returns_durable_not_allowed_for_teammate() {
    let svc = InMemoryTriggerScheduler::new(true);
    let t = task(
        "t-1",
        TriggerScope::Durable,
        TriggerOwner::Teammate("alice".to_string()),
    );

    let err = svc.create(t).await.expect_err("should be denied");
    assert!(
        matches!(err, TriggerError::DurableNotAllowedForTeammate),
        "expected DurableNotAllowedForTeammate, got: {err}"
    );
    assert_eq!(
        err.to_string(),
        "durable tasks not allowed for teammates",
        "Display impl must match contract"
    );
}

/// AC: invalid cron expression returns InvalidCron.
///
/// The contract requires cron validation before any task creation.
#[tokio::test]
async fn contract_invalid_cron_returns_invalid_cron() {
    let svc = InMemoryTriggerScheduler::new(true);
    let mut t = task("t-2", TriggerScope::SessionOnly, TriggerOwner::MainAgent);
    t.cron = "this is not cron".to_string();

    let err = svc.create(t).await.expect_err("should reject bad cron");
    assert!(
        matches!(err, TriggerError::InvalidCron),
        "expected InvalidCron, got: {err}"
    );
    assert_eq!(err.to_string(), "invalid cron");
}

/// AC: owner mismatch on delete returns OwnershipViolation.
///
/// Only the owner of a task may delete it.
#[tokio::test]
async fn contract_owner_mismatch_delete_returns_ownership_violation() {
    let svc = InMemoryTriggerScheduler::new(true);
    let t = task(
        "t-3",
        TriggerScope::SessionOnly,
        TriggerOwner::Teammate("alice".to_string()),
    );
    svc.create(t).await.expect("create");

    let err = svc
        .delete("t-3", TriggerOwner::Teammate("bob".to_string()))
        .await
        .expect_err("owner mismatch should fail");
    assert!(
        matches!(err, TriggerError::OwnershipViolation),
        "expected OwnershipViolation, got: {err}"
    );
    assert_eq!(err.to_string(), "ownership violation");

    // Correct owner can still delete
    svc.delete("t-3", TriggerOwner::Teammate("alice".to_string()))
        .await
        .expect("correct owner should succeed");
}

// ── §8.2 Integration tests ──────────────────────────────────────────────

/// AC: session-only task survives runtime tick but not process restart.
///
/// Session-only tasks are kept in memory and lost on restart.
#[tokio::test]
async fn integration_session_only_survives_runtime_not_restart() {
    let svc = InMemoryTriggerScheduler::new(true);
    let t = task("s-1", TriggerScope::SessionOnly, TriggerOwner::MainAgent);
    svc.create(t).await.expect("create");

    // Survives within same runtime
    let tasks = svc.list().await.expect("list");
    assert_eq!(tasks.len(), 1, "session task visible in same runtime");
    assert_eq!(tasks[0].id, "s-1");

    // Lost after restart
    let restarted = svc.restart();
    let after = restarted.list().await.expect("list after restart");
    assert!(
        after.is_empty(),
        "session-only task must not survive restart"
    );
}

/// AC: durable task reloads from store on restart.
///
/// Durable tasks persist to the durable store and reload on restart.
#[tokio::test]
async fn integration_durable_task_reloads_from_store_on_restart() {
    let svc = InMemoryTriggerScheduler::new(true);
    let t = task("d-1", TriggerScope::Durable, TriggerOwner::MainAgent);
    svc.create(t).await.expect("create durable");

    let restarted = svc.restart();
    let tasks = restarted.list().await.expect("list after restart");
    assert_eq!(tasks.len(), 1, "durable task must survive restart");
    assert_eq!(tasks[0].id, "d-1");
    assert_eq!(tasks[0].scope, TriggerScope::Durable);
}

/// AC: remote dispatch with gate off returns GateDisabled.
///
/// When the remote dispatch feature gate is disabled, all dispatch calls fail.
#[tokio::test]
async fn integration_remote_dispatch_gate_off_returns_gate_disabled() {
    let svc = MockRemoteDispatch::gate_disabled();

    let err = svc
        .dispatch(serde_json::json!({"task_id": "t-1"}))
        .await
        .expect_err("gate should block");
    assert!(
        matches!(err, TriggerError::GateDisabled),
        "expected GateDisabled, got: {err}"
    );
    assert_eq!(err.to_string(), "gate disabled");
}

// ── §8.3 CLI behaviour tests ────────────────────────────────────────────

/// AC: list shows scope and owner consistently.
///
/// Tasks of different scopes and owners are listed with correct attributes.
#[tokio::test]
async fn cli_list_shows_scope_and_owner_consistently() {
    let svc = InMemoryTriggerScheduler::new(true);

    // Create tasks with varying scope and owner
    svc.create(task(
        "a-1",
        TriggerScope::SessionOnly,
        TriggerOwner::MainAgent,
    ))
    .await
    .expect("create");
    svc.create(task(
        "b-2",
        TriggerScope::SessionOnly,
        TriggerOwner::Teammate("alice".to_string()),
    ))
    .await
    .expect("create");
    svc.create(task("c-3", TriggerScope::Durable, TriggerOwner::TeamLead))
        .await
        .expect("create");

    let tasks = svc.list().await.expect("list");
    assert_eq!(tasks.len(), 3);

    // Sorted by id
    assert_eq!(tasks[0].id, "a-1");
    assert_eq!(tasks[0].scope, TriggerScope::SessionOnly);
    assert_eq!(tasks[0].owner, TriggerOwner::MainAgent);

    assert_eq!(tasks[1].id, "b-2");
    assert_eq!(tasks[1].scope, TriggerScope::SessionOnly);
    assert_eq!(tasks[1].owner, TriggerOwner::Teammate("alice".to_string()));

    assert_eq!(tasks[2].id, "c-3");
    assert_eq!(tasks[2].scope, TriggerScope::Durable);
    assert_eq!(tasks[2].owner, TriggerOwner::TeamLead);

    // Verify scope and owner serialize consistently (round-trip)
    for t in &tasks {
        let json = serde_json::to_value(t).expect("serialize");
        let back: TriggerTask = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back.scope, t.scope, "scope round-trip for {}", t.id);
        assert_eq!(back.owner, t.owner, "owner round-trip for {}", t.id);
    }
}

/// AC: delete reports deterministic reason on ownership error.
///
/// The error type and message are stable across calls.
#[tokio::test]
async fn cli_delete_reports_deterministic_reason_on_ownership_error() {
    let svc = InMemoryTriggerScheduler::new(true);
    let t = task(
        "owned-1",
        TriggerScope::SessionOnly,
        TriggerOwner::Teammate("alice".to_string()),
    );
    svc.create(t).await.expect("create");

    // Multiple attempts produce the same error variant and message
    let err1 = svc
        .delete("owned-1", TriggerOwner::Teammate("bob".to_string()))
        .await
        .expect_err("mismatch");
    let err2 = svc
        .delete("owned-1", TriggerOwner::MainAgent)
        .await
        .expect_err("mismatch");

    assert!(matches!(err1, TriggerError::OwnershipViolation));
    assert!(matches!(err2, TriggerError::OwnershipViolation));
    assert_eq!(
        err1.to_string(),
        err2.to_string(),
        "error message must be deterministic across callers"
    );
    assert_eq!(err1.to_string(), "ownership violation");
}

/// AC: remote trigger failure prints non-leaky error class.
///
/// Transport-level details (HTTP status, internal messages) must not leak
/// through the error type's Display impl.
#[tokio::test]
async fn cli_remote_trigger_failure_prints_non_leaky_error_class() {
    let svc = MockRemoteDispatch::with_transport_error("http 500: secret internal detail");

    let err = svc
        .dispatch(serde_json::json!({"task_id": "t-1"}))
        .await
        .expect_err("transport failure");

    assert!(
        matches!(err, TriggerError::UpstreamRemoteError),
        "expected UpstreamRemoteError, got: {err}"
    );

    let msg = err.to_string();
    assert_eq!(msg, "upstream remote error", "must use stable error class");
    assert!(!msg.contains("500"), "HTTP status must not leak: '{msg}'");
    assert!(
        !msg.contains("secret"),
        "internal details must not leak: '{msg}'"
    );
}
