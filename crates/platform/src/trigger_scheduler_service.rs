use async_trait::async_trait;
use ccode_application::spec_contracts::{
    TriggerError, TriggerOwner, TriggerSchedulerService, TriggerScope, TriggerTask,
};
use ccode_state_store::TriggerTaskStateStore;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy)]
pub struct TriggerSchedulerPolicy {
    pub scheduler_enabled: bool,
    pub durable_enabled: bool,
}

impl Default for TriggerSchedulerPolicy {
    fn default() -> Self {
        Self {
            scheduler_enabled: true,
            durable_enabled: true,
        }
    }
}

pub struct LocalTriggerSchedulerService {
    store: Arc<dyn TriggerTaskStateStore>,
    policy: TriggerSchedulerPolicy,
    tasks: Mutex<HashMap<String, TriggerTask>>,
}

impl LocalTriggerSchedulerService {
    pub async fn new(
        store: Arc<dyn TriggerTaskStateStore>,
        policy: TriggerSchedulerPolicy,
    ) -> Result<Self, TriggerError> {
        let mut tasks = HashMap::new();
        let stored = store
            .load_trigger_tasks()
            .await
            .map_err(|_| TriggerError::StorageError)?;

        for raw in stored {
            let task: TriggerTask =
                serde_json::from_value(raw).map_err(|_| TriggerError::StorageError)?;
            tasks.insert(task.id.clone(), task);
        }

        Ok(Self {
            store,
            policy,
            tasks: Mutex::new(tasks),
        })
    }

    fn ensure_enabled(&self) -> Result<(), TriggerError> {
        if self.policy.scheduler_enabled {
            Ok(())
        } else {
            Err(TriggerError::GateDisabled)
        }
    }

    fn validate_create(&self, task: &TriggerTask) -> Result<(), TriggerError> {
        if ccode_cron::validate(&task.cron).is_err() {
            return Err(TriggerError::InvalidCron);
        }

        if task.scope == TriggerScope::Durable {
            if !self.policy.durable_enabled {
                return Err(TriggerError::GateDisabled);
            }
            if !task.durable_intent {
                return Err(TriggerError::Unauthorized);
            }
            if matches!(task.owner, TriggerOwner::Teammate(_)) {
                return Err(TriggerError::DurableNotAllowedForTeammate);
            }
        }

        Ok(())
    }
}

#[async_trait]
impl TriggerSchedulerService for LocalTriggerSchedulerService {
    async fn create(&self, task: TriggerTask) -> Result<TriggerTask, TriggerError> {
        self.ensure_enabled()?;
        self.validate_create(&task)?;

        if task.scope == TriggerScope::Durable {
            let data = serde_json::to_value(&task).map_err(|_| TriggerError::StorageError)?;
            self.store
                .save_trigger_task(&task.id, &data)
                .await
                .map_err(|_| TriggerError::StorageError)?;
        }

        self.tasks
            .lock()
            .map_err(|_| TriggerError::StorageError)?
            .insert(task.id.clone(), task.clone());

        Ok(task)
    }

    async fn list(&self) -> Result<Vec<TriggerTask>, TriggerError> {
        self.ensure_enabled()?;
        let tasks = self.tasks.lock().map_err(|_| TriggerError::StorageError)?;
        let mut listed: Vec<TriggerTask> = tasks.values().cloned().collect();
        listed.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(listed)
    }

    async fn delete(&self, id: &str, actor: TriggerOwner) -> Result<(), TriggerError> {
        self.ensure_enabled()?;
        let found = {
            let tasks = self.tasks.lock().map_err(|_| TriggerError::StorageError)?;
            tasks.get(id).cloned()
        };

        let task = match found {
            Some(task) => task,
            None => return Err(TriggerError::Unauthorized),
        };

        if task.owner != actor {
            return Err(TriggerError::OwnershipViolation);
        }

        if task.scope == TriggerScope::Durable {
            self.store
                .delete_trigger_task(id)
                .await
                .map_err(|_| TriggerError::StorageError)?;
        }

        self.tasks
            .lock()
            .map_err(|_| TriggerError::StorageError)?
            .remove(id);
        Ok(())
    }
}
