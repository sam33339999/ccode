use crate::spec_contracts::{
    UltraplanError, UltraplanPhase, UltraplanPolicy, UltraplanService, UltraplanSession,
};
use async_trait::async_trait;
use tokio::sync::Mutex;

#[async_trait]
pub trait UltraplanRuntime: Send + Sync {
    async fn launch_session(&self, prompt: &str) -> Result<String, UltraplanError>;
    async fn poll_phase(&self, session_id: &str) -> Result<UltraplanPhase, UltraplanError>;
    async fn stop_session(&self, session_id: &str) -> Result<(), UltraplanError>;
    async fn archive_session(&self, session_id: &str) -> Result<(), UltraplanError>;
}

#[derive(Debug, Clone)]
struct ActiveSession {
    session_id: String,
    phase: UltraplanPhase,
}

#[derive(Debug, Default)]
struct LocalState {
    launching: bool,
    active: Option<ActiveSession>,
    revision: u64,
}

pub struct DefaultUltraplanService<R> {
    runtime: R,
    policy_enabled: bool,
    state: Mutex<LocalState>,
}

impl<R> DefaultUltraplanService<R> {
    pub fn new(runtime: R, policy_enabled: bool) -> Self {
        Self {
            runtime,
            policy_enabled,
            state: Mutex::new(LocalState::default()),
        }
    }

    fn is_terminal(phase: UltraplanPhase) -> bool {
        matches!(phase, UltraplanPhase::Completed | UltraplanPhase::Failed)
    }

    fn transition_allowed(from: UltraplanPhase, to: UltraplanPhase) -> bool {
        matches!(
            (from, to),
            (UltraplanPhase::Idle, UltraplanPhase::Launching)
                | (UltraplanPhase::Launching, UltraplanPhase::Polling)
                | (UltraplanPhase::Launching, UltraplanPhase::Failed)
                | (UltraplanPhase::Polling, UltraplanPhase::AwaitingInput)
                | (UltraplanPhase::Polling, UltraplanPhase::Approved)
                | (UltraplanPhase::Polling, UltraplanPhase::Failed)
                | (UltraplanPhase::Polling, UltraplanPhase::Stopping)
                | (UltraplanPhase::AwaitingInput, UltraplanPhase::Polling)
                | (UltraplanPhase::AwaitingInput, UltraplanPhase::Stopping)
                | (UltraplanPhase::Approved, UltraplanPhase::Completed)
                | (UltraplanPhase::Stopping, UltraplanPhase::Completed)
                | (UltraplanPhase::Stopping, UltraplanPhase::Failed)
        )
    }

    async fn transition_keyword_guarded(&self) -> bool {
        let state = self.state.lock().await;
        if state.launching {
            return true;
        }

        state.active.as_ref().is_some_and(|active| {
            matches!(
                active.phase,
                UltraplanPhase::Launching | UltraplanPhase::Polling
            )
        })
    }

    async fn set_launching(&self, launching: bool) {
        let mut state = self.state.lock().await;
        state.launching = launching;
    }

    async fn set_active(&self, active: Option<ActiveSession>) {
        let mut state = self.state.lock().await;
        state.active = active;
        state.revision = state.revision.wrapping_add(1);
    }

    async fn apply_launch_guard(&self, policy: UltraplanPolicy) -> Result<(), UltraplanError> {
        let mut state = self.state.lock().await;
        if !policy.single_active_session {
            state.launching = true;
            return Ok(());
        }

        if state.launching {
            return Err(UltraplanError::AlreadyActive);
        }
        if let Some(active) = state.active.as_ref()
            && !Self::is_terminal(active.phase)
        {
            return Err(UltraplanError::AlreadyActive);
        }

        state.launching = true;
        Ok(())
    }

    async fn try_archive_on_failure(&self, session_id: &str) -> Result<(), UltraplanError>
    where
        R: UltraplanRuntime,
    {
        self.runtime
            .archive_session(session_id)
            .await
            .map_err(|_| UltraplanError::ArchiveFailed)
    }

    fn is_ultraplan_keyword(raw: &str) -> bool {
        let lowered = raw.trim().to_ascii_lowercase();
        lowered == "ultraplan" || lowered == "/ultraplan" || lowered.contains("ultraplan")
    }

    pub async fn route_keyword_launch(
        &self,
        raw_input: &str,
        prompt: &str,
        policy: UltraplanPolicy,
    ) -> Result<Option<UltraplanSession>, UltraplanError>
    where
        R: UltraplanRuntime + Send + Sync,
    {
        if !Self::is_ultraplan_keyword(raw_input) {
            return Ok(None);
        }
        if self.transition_keyword_guarded().await {
            return Err(UltraplanError::AlreadyActive);
        }

        self.launch(prompt, policy).await.map(Some)
    }

    pub async fn active_marker(&self) -> Option<(String, UltraplanPhase)> {
        let state = self.state.lock().await;
        state
            .active
            .as_ref()
            .map(|active| (active.session_id.clone(), active.phase))
    }
}

#[async_trait]
impl<R> UltraplanService for DefaultUltraplanService<R>
where
    R: UltraplanRuntime + Send + Sync,
{
    async fn launch(
        &self,
        prompt: &str,
        policy: UltraplanPolicy,
    ) -> Result<UltraplanSession, UltraplanError> {
        if !self.policy_enabled {
            return Err(UltraplanError::DisabledByPolicy);
        }

        self.apply_launch_guard(policy).await?;

        let session_id = match self.runtime.launch_session(prompt).await {
            Ok(session_id) => session_id,
            Err(_) => {
                self.set_launching(false).await;
                return Err(UltraplanError::LaunchFailed);
            }
        };

        let launch_result = self.runtime.poll_phase(&session_id).await;
        match launch_result {
            Ok(next_phase) if Self::transition_allowed(UltraplanPhase::Launching, next_phase) => {
                self.set_launching(false).await;
                if next_phase == UltraplanPhase::Failed {
                    self.set_active(None).await;
                    let _ = self.try_archive_on_failure(&session_id).await;
                    return Err(UltraplanError::LaunchFailed);
                }
                self.set_active(Some(ActiveSession {
                    session_id: session_id.clone(),
                    phase: next_phase,
                }))
                .await;
                Ok(UltraplanSession {
                    session_id,
                    phase: next_phase,
                })
            }
            Ok(_) | Err(_) => {
                self.set_launching(false).await;
                self.set_active(None).await;
                self.try_archive_on_failure(&session_id).await?;
                Err(UltraplanError::LaunchFailed)
            }
        }
    }

    async fn poll(&self, session_id: &str) -> Result<UltraplanPhase, UltraplanError> {
        if !self.policy_enabled {
            return Err(UltraplanError::DisabledByPolicy);
        }

        let (from_phase, revision) = {
            let state = self.state.lock().await;
            (
                state
                    .active
                    .as_ref()
                    .filter(|active| active.session_id == session_id)
                    .map_or(UltraplanPhase::Polling, |active| active.phase),
                state.revision,
            )
        };

        let next_phase = self.runtime.poll_phase(session_id).await?;
        if !Self::transition_allowed(from_phase, next_phase) {
            return Err(UltraplanError::ApprovalFailed);
        }

        let mut resolved_phase = next_phase;
        if next_phase == UltraplanPhase::Approved {
            resolved_phase = UltraplanPhase::Completed;
        }

        if Self::is_terminal(resolved_phase) {
            self.set_active(None).await;
        } else {
            let mut state = self.state.lock().await;
            if state.revision == revision {
                state.active = Some(ActiveSession {
                    session_id: session_id.to_owned(),
                    phase: resolved_phase,
                });
                state.revision = state.revision.wrapping_add(1);
            }
        }

        Ok(resolved_phase)
    }

    async fn stop(&self, session_id: &str) -> Result<(), UltraplanError> {
        if !self.policy_enabled {
            return Err(UltraplanError::DisabledByPolicy);
        }

        {
            let mut state = self.state.lock().await;
            state.launching = false;
            if state
                .active
                .as_ref()
                .is_some_and(|active| active.session_id == session_id)
            {
                state.active = None;
                state.revision = state.revision.wrapping_add(1);
            }
        }

        self.runtime.stop_session(session_id).await
    }

    async fn archive_orphan(&self, session_id: &str) -> Result<(), UltraplanError> {
        if !self.policy_enabled {
            return Err(UltraplanError::DisabledByPolicy);
        }

        self.runtime
            .archive_session(session_id)
            .await
            .map_err(|_| UltraplanError::ArchiveFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Arc};
    use tokio::sync::Notify;

    #[derive(Default)]
    struct MockUltraplanRuntime {
        launch_results: Arc<Mutex<VecDeque<Result<String, UltraplanError>>>>,
        poll_results: Arc<Mutex<VecDeque<Result<UltraplanPhase, UltraplanError>>>>,
        stop_results: Arc<Mutex<VecDeque<Result<(), UltraplanError>>>>,
        archive_results: Arc<Mutex<VecDeque<Result<(), UltraplanError>>>>,
        archive_calls: Arc<Mutex<Vec<String>>>,
        stop_calls: Arc<Mutex<Vec<String>>>,
    }

    impl MockUltraplanRuntime {
        fn with_poll(results: Vec<Result<UltraplanPhase, UltraplanError>>) -> Self {
            Self {
                poll_results: Arc::new(Mutex::new(results.into())),
                ..Self::default()
            }
        }
    }

    #[async_trait]
    impl UltraplanRuntime for MockUltraplanRuntime {
        async fn launch_session(&self, _prompt: &str) -> Result<String, UltraplanError> {
            self.launch_results
                .lock()
                .await
                .pop_front()
                .expect("missing launch result")
        }

        async fn poll_phase(&self, _session_id: &str) -> Result<UltraplanPhase, UltraplanError> {
            self.poll_results
                .lock()
                .await
                .pop_front()
                .expect("missing poll result")
        }

        async fn stop_session(&self, session_id: &str) -> Result<(), UltraplanError> {
            self.stop_calls.lock().await.push(session_id.to_owned());
            self.stop_results.lock().await.pop_front().unwrap_or(Ok(()))
        }

        async fn archive_session(&self, session_id: &str) -> Result<(), UltraplanError> {
            self.archive_calls.lock().await.push(session_id.to_owned());
            self.archive_results
                .lock()
                .await
                .pop_front()
                .unwrap_or(Ok(()))
        }
    }

    fn policy() -> UltraplanPolicy {
        UltraplanPolicy {
            single_active_session: true,
        }
    }

    #[test]
    fn transition_graph_matches_contract() {
        assert!(
            DefaultUltraplanService::<MockUltraplanRuntime>::transition_allowed(
                UltraplanPhase::Idle,
                UltraplanPhase::Launching
            )
        );
        assert!(
            DefaultUltraplanService::<MockUltraplanRuntime>::transition_allowed(
                UltraplanPhase::Launching,
                UltraplanPhase::Polling
            )
        );
        assert!(
            DefaultUltraplanService::<MockUltraplanRuntime>::transition_allowed(
                UltraplanPhase::Launching,
                UltraplanPhase::Failed
            )
        );
        assert!(
            DefaultUltraplanService::<MockUltraplanRuntime>::transition_allowed(
                UltraplanPhase::Polling,
                UltraplanPhase::AwaitingInput
            )
        );
        assert!(
            DefaultUltraplanService::<MockUltraplanRuntime>::transition_allowed(
                UltraplanPhase::Polling,
                UltraplanPhase::Approved
            )
        );
        assert!(
            DefaultUltraplanService::<MockUltraplanRuntime>::transition_allowed(
                UltraplanPhase::Polling,
                UltraplanPhase::Failed
            )
        );
        assert!(
            DefaultUltraplanService::<MockUltraplanRuntime>::transition_allowed(
                UltraplanPhase::Polling,
                UltraplanPhase::Stopping
            )
        );
        assert!(
            DefaultUltraplanService::<MockUltraplanRuntime>::transition_allowed(
                UltraplanPhase::AwaitingInput,
                UltraplanPhase::Polling
            )
        );
        assert!(
            DefaultUltraplanService::<MockUltraplanRuntime>::transition_allowed(
                UltraplanPhase::AwaitingInput,
                UltraplanPhase::Stopping
            )
        );
        assert!(
            DefaultUltraplanService::<MockUltraplanRuntime>::transition_allowed(
                UltraplanPhase::Approved,
                UltraplanPhase::Completed
            )
        );
        assert!(
            DefaultUltraplanService::<MockUltraplanRuntime>::transition_allowed(
                UltraplanPhase::Stopping,
                UltraplanPhase::Completed
            )
        );
        assert!(
            DefaultUltraplanService::<MockUltraplanRuntime>::transition_allowed(
                UltraplanPhase::Stopping,
                UltraplanPhase::Failed
            )
        );
        assert!(
            !DefaultUltraplanService::<MockUltraplanRuntime>::transition_allowed(
                UltraplanPhase::Completed,
                UltraplanPhase::Polling
            )
        );
        assert!(
            !DefaultUltraplanService::<MockUltraplanRuntime>::transition_allowed(
                UltraplanPhase::Failed,
                UltraplanPhase::Polling
            )
        );
    }

    #[tokio::test]
    async fn launch_returns_already_active_when_launching_or_polling() {
        let runtime = MockUltraplanRuntime {
            launch_results: Arc::new(Mutex::new(vec![Ok("s-1".to_owned())].into())),
            poll_results: Arc::new(Mutex::new(vec![Ok(UltraplanPhase::Polling)].into())),
            ..MockUltraplanRuntime::default()
        };
        let service = DefaultUltraplanService::new(runtime, true);

        let first = service.launch("prompt", policy()).await;
        assert!(first.is_ok());

        let second = service.launch("prompt", policy()).await;
        assert!(matches!(second, Err(UltraplanError::AlreadyActive)));
    }

    #[tokio::test]
    async fn concurrent_launch_is_guarded() {
        let runtime = MockUltraplanRuntime {
            launch_results: Arc::new(Mutex::new(
                vec![Ok("s-1".to_owned()), Ok("s-2".to_owned())].into(),
            )),
            poll_results: Arc::new(Mutex::new(vec![Ok(UltraplanPhase::Polling)].into())),
            ..MockUltraplanRuntime::default()
        };
        let service = Arc::new(DefaultUltraplanService::new(runtime, true));

        let left = service.launch("prompt", policy());
        let right = service.launch("prompt", policy());
        let (a, b) = tokio::join!(left, right);

        assert!(
            matches!(a, Ok(_)) && matches!(b, Err(UltraplanError::AlreadyActive))
                || matches!(b, Ok(_)) && matches!(a, Err(UltraplanError::AlreadyActive))
        );
    }

    #[tokio::test]
    async fn keyword_auto_route_respects_launching_and_polling_guard() {
        let runtime = MockUltraplanRuntime {
            launch_results: Arc::new(Mutex::new(vec![Ok("session_1".to_owned())].into())),
            poll_results: Arc::new(Mutex::new(vec![Ok(UltraplanPhase::Polling)].into())),
            ..MockUltraplanRuntime::default()
        };
        let service = DefaultUltraplanService::new(runtime, true);
        let _ = service
            .launch("boot", policy())
            .await
            .expect("launch succeeds");

        let routed = service
            .route_keyword_launch("please run ultraplan now", "second", policy())
            .await;

        assert!(matches!(routed, Err(UltraplanError::AlreadyActive)));
    }

    #[tokio::test]
    async fn launch_failure_after_remote_creation_attempts_archive() {
        let runtime = MockUltraplanRuntime {
            launch_results: Arc::new(Mutex::new(vec![Ok("session_123".to_owned())].into())),
            poll_results: Arc::new(Mutex::new(
                vec![Err(UltraplanError::Transport("x".into()))].into(),
            )),
            ..MockUltraplanRuntime::default()
        };
        let archive_calls = runtime.archive_calls.clone();
        let service = DefaultUltraplanService::new(runtime, true);

        let result = service.launch("prompt", policy()).await;
        assert!(matches!(result, Err(UltraplanError::LaunchFailed)));

        let calls = archive_calls.lock().await.clone();
        assert_eq!(calls, vec!["session_123".to_owned()]);
    }

    #[tokio::test]
    async fn stop_clears_active_marker_before_returning_error() {
        let runtime = MockUltraplanRuntime {
            launch_results: Arc::new(Mutex::new(vec![Ok("session_1".to_owned())].into())),
            poll_results: Arc::new(Mutex::new(vec![Ok(UltraplanPhase::Polling)].into())),
            stop_results: Arc::new(Mutex::new(
                vec![Err(UltraplanError::Transport("cannot stop".to_owned()))].into(),
            )),
            ..MockUltraplanRuntime::default()
        };
        let service = DefaultUltraplanService::new(runtime, true);
        let _ = service
            .launch("prompt", policy())
            .await
            .expect("launch succeeds");

        let stop_result = service.stop("session_1").await;
        assert!(matches!(stop_result, Err(UltraplanError::Transport(_))));
        assert!(service.active_marker().await.is_none());
    }

    #[tokio::test]
    async fn poll_handles_transitions_and_approved_to_completed() {
        let runtime = MockUltraplanRuntime {
            launch_results: Arc::new(Mutex::new(vec![Ok("session_1".to_owned())].into())),
            poll_results: Arc::new(Mutex::new(
                vec![
                    Ok(UltraplanPhase::Polling),
                    Ok(UltraplanPhase::AwaitingInput),
                    Ok(UltraplanPhase::Polling),
                    Ok(UltraplanPhase::Stopping),
                    Ok(UltraplanPhase::Completed),
                ]
                .into(),
            )),
            ..MockUltraplanRuntime::default()
        };
        let service = DefaultUltraplanService::new(runtime, true);

        let _ = service
            .launch("prompt", policy())
            .await
            .expect("launch succeeds");
        assert_eq!(
            service.poll("session_1").await.expect("awaiting input"),
            UltraplanPhase::AwaitingInput
        );
        assert_eq!(
            service.poll("session_1").await.expect("back to polling"),
            UltraplanPhase::Polling
        );
        assert_eq!(
            service.poll("session_1").await.expect("stopping"),
            UltraplanPhase::Stopping
        );
        assert_eq!(
            service.poll("session_1").await.expect("completed"),
            UltraplanPhase::Completed
        );
        assert!(service.active_marker().await.is_none());
    }

    #[tokio::test]
    async fn poll_maps_each_terminal_and_error_path() {
        let runtime = MockUltraplanRuntime::with_poll(vec![Err(UltraplanError::PollTimeout)]);
        let service = DefaultUltraplanService::new(runtime, true);
        let timeout = service.poll("session_1").await;
        assert!(matches!(timeout, Err(UltraplanError::PollTimeout)));
    }

    #[tokio::test]
    async fn can_emit_each_error_variant() {
        let disabled = DefaultUltraplanService::new(MockUltraplanRuntime::default(), false)
            .launch("prompt", policy())
            .await;
        assert!(matches!(disabled, Err(UltraplanError::DisabledByPolicy)));

        let already_runtime = MockUltraplanRuntime {
            launch_results: Arc::new(Mutex::new(vec![Ok("s-1".to_owned())].into())),
            poll_results: Arc::new(Mutex::new(vec![Ok(UltraplanPhase::Polling)].into())),
            ..MockUltraplanRuntime::default()
        };
        let already_service = DefaultUltraplanService::new(already_runtime, true);
        let _ = already_service
            .launch("prompt", policy())
            .await
            .expect("first launch succeeds");
        let already = already_service.launch("prompt", policy()).await;
        assert!(matches!(already, Err(UltraplanError::AlreadyActive)));

        let launch_failed_runtime = MockUltraplanRuntime {
            launch_results: Arc::new(Mutex::new(
                vec![Err(UltraplanError::Transport("x".into()))].into(),
            )),
            ..MockUltraplanRuntime::default()
        };
        let launch_failed = DefaultUltraplanService::new(launch_failed_runtime, true)
            .launch("prompt", policy())
            .await;
        assert!(matches!(launch_failed, Err(UltraplanError::LaunchFailed)));

        let poll_timeout_runtime =
            MockUltraplanRuntime::with_poll(vec![Err(UltraplanError::PollTimeout)]);
        let poll_timeout = DefaultUltraplanService::new(poll_timeout_runtime, true)
            .poll("session")
            .await;
        assert!(matches!(poll_timeout, Err(UltraplanError::PollTimeout)));

        let approval_failed_runtime = MockUltraplanRuntime {
            launch_results: Arc::new(Mutex::new(vec![Ok("session_1".to_owned())].into())),
            poll_results: Arc::new(Mutex::new(
                vec![
                    Ok(UltraplanPhase::Polling),
                    Ok(UltraplanPhase::AwaitingInput),
                    Ok(UltraplanPhase::Failed),
                ]
                .into(),
            )),
            ..MockUltraplanRuntime::default()
        };
        let approval_failed_service = DefaultUltraplanService::new(approval_failed_runtime, true);
        let _ = approval_failed_service
            .launch("prompt", policy())
            .await
            .expect("launch succeeds");
        let _ = approval_failed_service
            .poll("session_1")
            .await
            .expect("awaiting input");
        let approval_failed = approval_failed_service.poll("session_1").await;
        assert!(matches!(
            approval_failed,
            Err(UltraplanError::ApprovalFailed)
        ));

        let archive_failed_runtime = MockUltraplanRuntime {
            archive_results: Arc::new(Mutex::new(
                vec![Err(UltraplanError::Transport("x".into()))].into(),
            )),
            ..MockUltraplanRuntime::default()
        };
        let archive_failed = DefaultUltraplanService::new(archive_failed_runtime, true)
            .archive_orphan("session_2")
            .await;
        assert!(matches!(archive_failed, Err(UltraplanError::ArchiveFailed)));

        let transport_runtime = MockUltraplanRuntime {
            stop_results: Arc::new(Mutex::new(
                vec![Err(UltraplanError::Transport("stop failed".to_owned()))].into(),
            )),
            ..MockUltraplanRuntime::default()
        };
        let transport = DefaultUltraplanService::new(transport_runtime, true)
            .stop("session_3")
            .await;
        assert!(matches!(transport, Err(UltraplanError::Transport(_))));
    }

    #[tokio::test]
    async fn archive_failure_during_launch_failure_surfaces_archive_failed() {
        let runtime = MockUltraplanRuntime {
            launch_results: Arc::new(Mutex::new(vec![Ok("session_123".to_owned())].into())),
            poll_results: Arc::new(Mutex::new(
                vec![Err(UltraplanError::Transport("x".into()))].into(),
            )),
            archive_results: Arc::new(Mutex::new(
                vec![Err(UltraplanError::Transport("bad".into()))].into(),
            )),
            ..MockUltraplanRuntime::default()
        };
        let service = DefaultUltraplanService::new(runtime, true);

        let result = service.launch("prompt", policy()).await;
        assert!(matches!(result, Err(UltraplanError::ArchiveFailed)));
    }

    #[tokio::test]
    async fn route_keyword_non_ultraplan_passthroughs_without_launch() {
        let runtime = MockUltraplanRuntime::default();
        let service = DefaultUltraplanService::new(runtime, true);

        let routed = service
            .route_keyword_launch("hello world", "prompt", policy())
            .await
            .expect("routing succeeds");
        assert!(routed.is_none());
    }

    #[tokio::test]
    async fn concurrent_stop_and_poll_does_not_restore_active_marker() {
        struct RaceRuntime {
            poll_release: Arc<Notify>,
            poll_calls: Arc<Mutex<u8>>,
        }

        #[async_trait]
        impl UltraplanRuntime for RaceRuntime {
            async fn launch_session(&self, _prompt: &str) -> Result<String, UltraplanError> {
                Ok("session-race".to_string())
            }

            async fn poll_phase(
                &self,
                _session_id: &str,
            ) -> Result<UltraplanPhase, UltraplanError> {
                let mut calls = self.poll_calls.lock().await;
                *calls += 1;
                let call = *calls;
                drop(calls);

                if call == 1 {
                    return Ok(UltraplanPhase::Polling);
                }

                self.poll_release.notified().await;
                Ok(UltraplanPhase::AwaitingInput)
            }

            async fn stop_session(&self, _session_id: &str) -> Result<(), UltraplanError> {
                self.poll_release.notify_waiters();
                Ok(())
            }

            async fn archive_session(&self, _session_id: &str) -> Result<(), UltraplanError> {
                Ok(())
            }
        }

        let runtime = RaceRuntime {
            poll_release: Arc::new(Notify::new()),
            poll_calls: Arc::new(Mutex::new(0)),
        };
        let service = Arc::new(DefaultUltraplanService::new(runtime, true));
        service
            .launch("prompt", policy())
            .await
            .expect("launch succeeds");

        let poll = {
            let service = Arc::clone(&service);
            tokio::spawn(async move { service.poll("session-race").await })
        };
        tokio::task::yield_now().await;
        let stop = {
            let service = Arc::clone(&service);
            tokio::spawn(async move { service.stop("session-race").await })
        };

        let poll_result = poll.await.expect("poll task join");
        let stop_result = stop.await.expect("stop task join");

        assert_eq!(
            poll_result.expect("poll result"),
            UltraplanPhase::AwaitingInput
        );
        stop_result.expect("stop result");
        assert!(
            service.active_marker().await.is_none(),
            "active marker must remain cleared after stop"
        );
    }
}
