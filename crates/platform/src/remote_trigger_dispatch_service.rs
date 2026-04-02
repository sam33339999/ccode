use async_trait::async_trait;
use ccode_application::spec_contracts::{RemoteTriggerDispatchService, TriggerError};
use ccode_remote_runtime::ccr_client::{AuthContext, AuthContextProvider};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default)]
pub struct RemoteTriggerDispatchPolicy {
    pub remote_dispatch_enabled: bool,
}

#[async_trait]
pub trait RemoteTriggerTransport: Send + Sync {
    async fn dispatch(&self, payload: Value, auth: &AuthContext) -> Result<String, String>;
}

pub struct PlatformRemoteTriggerDispatchService<T, P> {
    transport: T,
    auth_provider: P,
    policy: RemoteTriggerDispatchPolicy,
}

impl<T, P> PlatformRemoteTriggerDispatchService<T, P>
where
    T: RemoteTriggerTransport,
    P: AuthContextProvider,
{
    pub fn new(transport: T, auth_provider: P, policy: RemoteTriggerDispatchPolicy) -> Self {
        Self {
            transport,
            auth_provider,
            policy,
        }
    }

    fn ensure_enabled(&self) -> Result<(), TriggerError> {
        if self.policy.remote_dispatch_enabled {
            Ok(())
        } else {
            Err(TriggerError::GateDisabled)
        }
    }

    fn validated_auth_context(&self) -> Result<AuthContext, TriggerError> {
        let auth = self.auth_provider.auth_context();

        let has_token = auth
            .token
            .as_ref()
            .map(|t| !t.trim().is_empty())
            .unwrap_or(false);
        let has_org = auth
            .org_id
            .as_ref()
            .map(|o| !o.trim().is_empty())
            .unwrap_or(false);

        if has_token && has_org {
            Ok(auth)
        } else {
            Err(TriggerError::Unauthorized)
        }
    }
}

#[async_trait]
impl<T, P> RemoteTriggerDispatchService for PlatformRemoteTriggerDispatchService<T, P>
where
    T: RemoteTriggerTransport,
    P: AuthContextProvider,
{
    async fn dispatch(&self, payload: Value) -> Result<String, TriggerError> {
        self.ensure_enabled()?;
        let auth = self.validated_auth_context()?;

        self.transport
            .dispatch(payload, &auth)
            .await
            .map_err(|_| TriggerError::UpstreamRemoteError)
    }
}
