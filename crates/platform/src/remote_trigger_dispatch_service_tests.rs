#[cfg(test)]
mod tests {
    use crate::{
        remote_trigger_dispatch_service::{
            PlatformRemoteTriggerDispatchService, RemoteTriggerDispatchPolicy,
            RemoteTriggerTransport,
        },
        trigger_scheduler_service::{LocalTriggerSchedulerService, TriggerSchedulerPolicy},
    };
    use ccode_application::spec_contracts::{
        RemoteTriggerDispatchService, TriggerError, TriggerOwner, TriggerSchedulerService,
        TriggerScope, TriggerTask,
    };
    use ccode_remote_runtime::ccr_client::{AuthContext, AuthContextProvider};
    use ccode_state_store::FileTriggerTaskStateStore;
    use serde_json::{Value, json};
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    #[derive(Debug, Clone)]
    struct TestAuthProvider {
        ctx: AuthContext,
    }

    impl AuthContextProvider for TestAuthProvider {
        fn auth_context(&self) -> AuthContext {
            self.ctx.clone()
        }
    }

    #[derive(Clone)]
    struct MockRemoteTransport {
        results: Arc<Mutex<VecDeque<Result<String, String>>>>,
    }

    impl MockRemoteTransport {
        fn with_results(results: Vec<Result<String, String>>) -> Self {
            Self {
                results: Arc::new(Mutex::new(results.into())),
            }
        }
    }

    #[async_trait::async_trait]
    impl RemoteTriggerTransport for MockRemoteTransport {
        async fn dispatch(&self, _payload: Value, _auth: &AuthContext) -> Result<String, String> {
            self.results
                .lock()
                .expect("lock")
                .pop_front()
                .expect("missing transport result")
        }
    }

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("ccode-us017-{label}-{ts}"));
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn task(id: &str) -> TriggerTask {
        TriggerTask {
            id: id.to_string(),
            cron: "0 9 * * *".to_string(),
            prompt: "do work".to_string(),
            scope: TriggerScope::SessionOnly,
            owner: TriggerOwner::MainAgent,
            durable_intent: false,
        }
    }

    #[tokio::test]
    async fn gate_off_returns_gate_disabled() {
        let service = PlatformRemoteTriggerDispatchService::new(
            MockRemoteTransport::with_results(vec![Ok("rid-1".to_string())]),
            TestAuthProvider {
                ctx: AuthContext {
                    token: Some("token".to_string()),
                    org_id: Some("org".to_string()),
                },
            },
            RemoteTriggerDispatchPolicy {
                remote_dispatch_enabled: false,
            },
        );

        let err = service
            .dispatch(json!({"task_id": "t-1"}))
            .await
            .expect_err("gate should block dispatch");

        assert!(matches!(err, TriggerError::GateDisabled));
    }

    #[tokio::test]
    async fn missing_auth_or_org_returns_unauthorized() {
        let missing_token = PlatformRemoteTriggerDispatchService::new(
            MockRemoteTransport::with_results(vec![Ok("rid-1".to_string())]),
            TestAuthProvider {
                ctx: AuthContext {
                    token: None,
                    org_id: Some("org".to_string()),
                },
            },
            RemoteTriggerDispatchPolicy {
                remote_dispatch_enabled: true,
            },
        );

        let token_err = missing_token
            .dispatch(json!({"task_id": "t-1"}))
            .await
            .expect_err("missing token should fail");
        assert!(matches!(token_err, TriggerError::Unauthorized));

        let missing_org = PlatformRemoteTriggerDispatchService::new(
            MockRemoteTransport::with_results(vec![Ok("rid-2".to_string())]),
            TestAuthProvider {
                ctx: AuthContext {
                    token: Some("token".to_string()),
                    org_id: None,
                },
            },
            RemoteTriggerDispatchPolicy {
                remote_dispatch_enabled: true,
            },
        );

        let org_err = missing_org
            .dispatch(json!({"task_id": "t-1"}))
            .await
            .expect_err("missing org should fail");
        assert!(matches!(org_err, TriggerError::Unauthorized));
    }

    #[tokio::test]
    async fn transport_errors_map_to_non_leaky_upstream_remote_error() {
        let service = PlatformRemoteTriggerDispatchService::new(
            MockRemoteTransport::with_results(vec![Err("http 500: leaked detail".to_string())]),
            TestAuthProvider {
                ctx: AuthContext {
                    token: Some("token".to_string()),
                    org_id: Some("org".to_string()),
                },
            },
            RemoteTriggerDispatchPolicy {
                remote_dispatch_enabled: true,
            },
        );

        let err = service
            .dispatch(json!({"task_id": "t-1"}))
            .await
            .expect_err("transport failure should map to stable upstream error");

        assert!(matches!(err, TriggerError::UpstreamRemoteError));
        assert_eq!(err.to_string(), "upstream remote error");
    }

    #[tokio::test]
    async fn local_scheduler_still_works_when_remote_dispatch_is_disabled() {
        let dir = unique_temp_dir("local-independent");
        let store = Arc::new(FileTriggerTaskStateStore::new(&dir).expect("store"));
        let scheduler = LocalTriggerSchedulerService::new(store, TriggerSchedulerPolicy::default())
            .await
            .expect("scheduler");

        scheduler
            .create(task("local-1"))
            .await
            .expect("create local");
        let local_tasks = scheduler.list().await.expect("list local");
        assert_eq!(local_tasks.len(), 1);

        let remote = PlatformRemoteTriggerDispatchService::new(
            MockRemoteTransport::with_results(vec![Ok("rid-1".to_string())]),
            TestAuthProvider {
                ctx: AuthContext {
                    token: Some("token".to_string()),
                    org_id: Some("org".to_string()),
                },
            },
            RemoteTriggerDispatchPolicy {
                remote_dispatch_enabled: false,
            },
        );

        let err = remote
            .dispatch(json!({"task_id": "local-1"}))
            .await
            .expect_err("remote dispatch should remain disabled");
        assert!(matches!(err, TriggerError::GateDisabled));

        let local_tasks_after = scheduler.list().await.expect("list local after");
        assert_eq!(local_tasks_after.len(), 1);
        assert_eq!(local_tasks_after[0].id, "local-1");
    }
}
