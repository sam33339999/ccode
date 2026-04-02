#[cfg(test)]
mod tests {
    use crate::trigger_scheduler_service::{LocalTriggerSchedulerService, TriggerSchedulerPolicy};
    use ccode_application::spec_contracts::{
        TriggerError, TriggerOwner, TriggerSchedulerService, TriggerScope, TriggerTask,
    };
    use ccode_state_store::FileTriggerTaskStateStore;

    fn task(id: &str, scope: TriggerScope, owner: TriggerOwner) -> TriggerTask {
        TriggerTask {
            id: id.to_string(),
            cron: "0 9 * * *".to_string(),
            prompt: "do work".to_string(),
            scope,
            owner,
            durable_intent: false,
        }
    }

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("ccode-us016-{label}-{ts}"));
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[tokio::test]
    async fn durable_persists_and_reloads_on_restart() {
        let dir = unique_temp_dir("durable-reload");
        let store = std::sync::Arc::new(FileTriggerTaskStateStore::new(&dir).expect("store"));

        let service =
            LocalTriggerSchedulerService::new(store.clone(), TriggerSchedulerPolicy::default())
                .await
                .expect("service");

        let mut durable = task("durable-1", TriggerScope::Durable, TriggerOwner::MainAgent);
        durable.durable_intent = true;
        service
            .create(durable.clone())
            .await
            .expect("create durable");

        let restarted = LocalTriggerSchedulerService::new(store, TriggerSchedulerPolicy::default())
            .await
            .expect("reload");

        let tasks = restarted.list().await.expect("list");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, durable.id);
        assert_eq!(tasks[0].scope, TriggerScope::Durable);
    }

    #[tokio::test]
    async fn session_only_survives_runtime_but_not_restart() {
        let dir = unique_temp_dir("session-only");
        let store = std::sync::Arc::new(FileTriggerTaskStateStore::new(&dir).expect("store"));

        let service =
            LocalTriggerSchedulerService::new(store.clone(), TriggerSchedulerPolicy::default())
                .await
                .expect("service");

        let session_task = task(
            "session-1",
            TriggerScope::SessionOnly,
            TriggerOwner::MainAgent,
        );
        service
            .create(session_task.clone())
            .await
            .expect("create session-only");

        let same_runtime = service.list().await.expect("list same runtime");
        assert_eq!(same_runtime.len(), 1);
        assert_eq!(same_runtime[0].id, session_task.id);

        let restarted = LocalTriggerSchedulerService::new(store, TriggerSchedulerPolicy::default())
            .await
            .expect("restart");
        let after_restart = restarted.list().await.expect("list after restart");
        assert!(after_restart.is_empty());
    }

    #[tokio::test]
    async fn teammate_cannot_create_durable() {
        let dir = unique_temp_dir("teammate-durable");
        let store = std::sync::Arc::new(FileTriggerTaskStateStore::new(&dir).expect("store"));
        let service = LocalTriggerSchedulerService::new(store, TriggerSchedulerPolicy::default())
            .await
            .expect("service");

        let mut durable = task(
            "durable-teammate",
            TriggerScope::Durable,
            TriggerOwner::Teammate("alice".to_string()),
        );
        durable.durable_intent = true;

        let err = service
            .create(durable)
            .await
            .expect_err("durable should be denied");
        assert!(matches!(err, TriggerError::DurableNotAllowedForTeammate));
    }

    #[tokio::test]
    async fn delete_checks_owner_identity() {
        let dir = unique_temp_dir("owner-delete");
        let store = std::sync::Arc::new(FileTriggerTaskStateStore::new(&dir).expect("store"));
        let service = LocalTriggerSchedulerService::new(store, TriggerSchedulerPolicy::default())
            .await
            .expect("service");

        let owned = task(
            "owned-1",
            TriggerScope::SessionOnly,
            TriggerOwner::Teammate("alice".to_string()),
        );
        service.create(owned).await.expect("create");

        let err = service
            .delete("owned-1", TriggerOwner::Teammate("bob".to_string()))
            .await
            .expect_err("owner mismatch should fail");
        assert!(matches!(err, TriggerError::OwnershipViolation));

        service
            .delete("owned-1", TriggerOwner::Teammate("alice".to_string()))
            .await
            .expect("owner match should delete");
    }

    #[tokio::test]
    async fn invalid_cron_returns_invalid_cron_error() {
        let dir = unique_temp_dir("invalid-cron");
        let store = std::sync::Arc::new(FileTriggerTaskStateStore::new(&dir).expect("store"));
        let service = LocalTriggerSchedulerService::new(store, TriggerSchedulerPolicy::default())
            .await
            .expect("service");

        let mut invalid = task(
            "invalid-1",
            TriggerScope::SessionOnly,
            TriggerOwner::MainAgent,
        );
        invalid.cron = "this is not cron".to_string();

        let err = service
            .create(invalid)
            .await
            .expect_err("invalid cron should fail");
        assert!(matches!(err, TriggerError::InvalidCron));
    }

    #[tokio::test]
    async fn durable_requires_explicit_user_intent() {
        let dir = unique_temp_dir("durable-intent");
        let store = std::sync::Arc::new(FileTriggerTaskStateStore::new(&dir).expect("store"));
        let service = LocalTriggerSchedulerService::new(store, TriggerSchedulerPolicy::default())
            .await
            .expect("service");

        let durable = task(
            "durable-no-intent",
            TriggerScope::Durable,
            TriggerOwner::MainAgent,
        );
        let err = service
            .create(durable)
            .await
            .expect_err("missing durable intent should fail");
        assert!(matches!(err, TriggerError::Unauthorized));
    }

    #[tokio::test]
    async fn gate_disabled_returns_gate_disabled() {
        let dir = unique_temp_dir("gate-disabled");
        let store = std::sync::Arc::new(FileTriggerTaskStateStore::new(&dir).expect("store"));
        let service = LocalTriggerSchedulerService::new(
            store,
            TriggerSchedulerPolicy {
                scheduler_enabled: false,
                durable_enabled: true,
            },
        )
        .await
        .expect("service");

        let err = service
            .list()
            .await
            .expect_err("list should be blocked when gate disabled");
        assert!(matches!(err, TriggerError::GateDisabled));
    }

    #[tokio::test]
    async fn storage_error_is_mapped_from_store_failures() {
        struct FailingStore;

        #[async_trait::async_trait]
        impl ccode_state_store::TriggerTaskStateStore for FailingStore {
            async fn load_trigger_tasks(
                &self,
            ) -> Result<Vec<serde_json::Value>, ccode_state_store::StateStoreError> {
                Ok(Vec::new())
            }

            async fn save_trigger_task(
                &self,
                _id: &str,
                _task: &serde_json::Value,
            ) -> Result<(), ccode_state_store::StateStoreError> {
                Err(ccode_state_store::StateStoreError::Storage(
                    "save boom".to_string(),
                ))
            }

            async fn delete_trigger_task(
                &self,
                _id: &str,
            ) -> Result<(), ccode_state_store::StateStoreError> {
                Ok(())
            }
        }

        let store = std::sync::Arc::new(FailingStore);
        let service = LocalTriggerSchedulerService::new(store, TriggerSchedulerPolicy::default())
            .await
            .expect("service");

        let mut durable = task(
            "durable-store",
            TriggerScope::Durable,
            TriggerOwner::MainAgent,
        );
        durable.durable_intent = true;

        let err = service
            .create(durable)
            .await
            .expect_err("storage failure should bubble up");
        assert!(matches!(err, TriggerError::StorageError));
    }

    #[test]
    fn upstream_remote_error_variant_exists() {
        let err = TriggerError::UpstreamRemoteError;
        assert_eq!(err.to_string(), "upstream remote error");
    }
}
