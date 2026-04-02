use crate::contracts::{
    ArchiveResult, CcrClient, CcrClientError, CreateRemoteSessionRequest, RemoteSessionState,
    RemoteSessionSummary,
};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const SESSION_PREFIX: &str = "session_";
const CSE_PREFIX: &str = "cse_";
const ORG_HEADER: &str = "x-org-id";

#[derive(Debug, Clone)]
pub struct HttpCcrClientConfig {
    pub base_url: String,
    pub timeout: Duration,
    pub max_retries: u32,
    pub retry_delay: Duration,
}

impl Default for HttpCcrClientConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:8080".to_string(),
            timeout: Duration::from_secs(10),
            max_retries: 2,
            retry_delay: Duration::from_millis(200),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AuthContext {
    pub token: Option<String>,
    pub org_id: Option<String>,
}

pub trait AuthContextProvider: Send + Sync {
    fn auth_context(&self) -> AuthContext;
}

#[derive(Debug, Clone, Default)]
pub struct StaticAuthContextProvider {
    ctx: AuthContext,
}

impl StaticAuthContextProvider {
    pub fn new(ctx: AuthContext) -> Self {
        Self { ctx }
    }
}

impl AuthContextProvider for StaticAuthContextProvider {
    fn auth_context(&self) -> AuthContext {
        self.ctx.clone()
    }
}

pub struct HttpCcrClient<P> {
    client: reqwest::Client,
    config: HttpCcrClientConfig,
    auth_provider: P,
}

impl<P> HttpCcrClient<P>
where
    P: AuthContextProvider,
{
    pub fn new(config: HttpCcrClientConfig, auth_provider: P) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
            auth_provider,
        }
    }

    fn sessions_endpoint(&self) -> String {
        format!("{}/sessions", self.config.base_url.trim_end_matches('/'))
    }

    fn session_endpoint(&self, session_id: &str) -> String {
        format!(
            "{}/{}",
            self.sessions_endpoint(),
            normalize_session_id_for_transport(session_id)
        )
    }

    fn add_auth_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let ctx = self.auth_provider.auth_context();
        let mut with_headers = builder;
        if let Some(token) = ctx.token {
            with_headers = with_headers.bearer_auth(token);
        }
        if let Some(org_id) = ctx.org_id {
            with_headers = with_headers.header(ORG_HEADER, org_id);
        }
        with_headers
    }

    async fn send_with_retry<F>(
        &self,
        mut make_request: F,
    ) -> Result<reqwest::Response, CcrClientError>
    where
        F: FnMut() -> reqwest::RequestBuilder,
    {
        let mut attempt = 0;
        loop {
            let send_result =
                tokio::time::timeout(self.config.timeout, make_request().send()).await;
            match send_result {
                Err(_) => {
                    if attempt < self.config.max_retries {
                        attempt += 1;
                        tokio::time::sleep(self.config.retry_delay).await;
                        continue;
                    }
                    return Err(CcrClientError::Timeout);
                }
                Ok(Err(err)) => {
                    let mapped = map_transport_error(
                        if err.is_timeout() {
                            TransportErrorKind::Timeout
                        } else {
                            TransportErrorKind::Http
                        },
                        err.to_string(),
                    );
                    if matches!(mapped, CcrClientError::Timeout)
                        && attempt < self.config.max_retries
                    {
                        attempt += 1;
                        tokio::time::sleep(self.config.retry_delay).await;
                        continue;
                    }
                    return Err(mapped);
                }
                Ok(Ok(resp)) => {
                    if should_retry_status(resp.status()) && attempt < self.config.max_retries {
                        attempt += 1;
                        tokio::time::sleep(self.config.retry_delay).await;
                        continue;
                    }
                    return Ok(resp);
                }
            }
        }
    }

    async fn decode_summary(
        resp: reqwest::Response,
    ) -> Result<RemoteSessionSummary, CcrClientError> {
        let payload: RemoteSessionSummaryWire = resp
            .json()
            .await
            .map_err(|e| map_transport_error(TransportErrorKind::InvalidPayload, e.to_string()))?;

        let state = parse_remote_state(&payload.state).ok_or(CcrClientError::InvalidPayload)?;
        Ok(RemoteSessionSummary {
            session_id: normalize_session_id_for_response(&payload.session_id),
            title: payload.title,
            environment_id: payload.environment_id,
            state,
        })
    }
}

#[async_trait::async_trait]
impl<P> CcrClient for HttpCcrClient<P>
where
    P: AuthContextProvider,
{
    async fn create(
        &self,
        req: CreateRemoteSessionRequest,
    ) -> Result<RemoteSessionSummary, CcrClientError> {
        let body = CreateRemoteSessionRequestWire::from(req);
        let endpoint = self.sessions_endpoint();
        let response = self
            .send_with_retry(|| self.add_auth_headers(self.client.post(&endpoint).json(&body)))
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(classify_status_error(status, body));
        }

        Self::decode_summary(response).await
    }

    async fn get(&self, session_id: &str) -> Result<RemoteSessionSummary, CcrClientError> {
        let endpoint = self.session_endpoint(session_id);
        let response = self
            .send_with_retry(|| self.add_auth_headers(self.client.get(&endpoint)))
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(classify_status_error(status, body));
        }

        Self::decode_summary(response).await
    }

    async fn archive(&self, session_id: &str) -> Result<ArchiveResult, CcrClientError> {
        let endpoint = format!("{}/archive", self.session_endpoint(session_id));
        let response = self
            .send_with_retry(|| self.add_auth_headers(self.client.post(&endpoint)))
            .await?;

        if response.status() == StatusCode::CONFLICT {
            return Ok(ArchiveResult::AlreadyArchived);
        }
        if response.status().is_success() {
            return Ok(ArchiveResult::Archived);
        }

        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        Err(classify_status_error(status, body))
    }

    async fn patch_title(&self, session_id: &str, title: &str) -> Result<(), CcrClientError> {
        let endpoint = self.session_endpoint(session_id);
        let body = PatchTitleWire {
            title: title.to_string(),
        };
        let response = self
            .send_with_retry(|| self.add_auth_headers(self.client.patch(&endpoint).json(&body)))
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            Err(classify_status_error(status, body))
        }
    }
}

pub fn normalize_session_id_for_transport(session_id: &str) -> String {
    if let Some(stripped) = session_id.strip_prefix(SESSION_PREFIX) {
        format!("{CSE_PREFIX}{stripped}")
    } else {
        session_id.to_string()
    }
}

pub fn normalize_session_id_for_response(session_id: &str) -> String {
    if let Some(stripped) = session_id.strip_prefix(CSE_PREFIX) {
        format!("{SESSION_PREFIX}{stripped}")
    } else {
        session_id.to_string()
    }
}

fn should_retry_status(status: StatusCode) -> bool {
    status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS
}

pub fn classify_status_error(status: u16, body: String) -> CcrClientError {
    match status {
        401 => CcrClientError::Unauthorized,
        403 => CcrClientError::Forbidden,
        _ => CcrClientError::Http(format!("status {status}: {body}")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportErrorKind {
    Timeout,
    InvalidPayload,
    Http,
}

pub fn map_transport_error(kind: TransportErrorKind, message: String) -> CcrClientError {
    match kind {
        TransportErrorKind::Timeout => CcrClientError::Timeout,
        TransportErrorKind::InvalidPayload => CcrClientError::InvalidPayload,
        TransportErrorKind::Http => CcrClientError::Http(message),
    }
}

fn parse_remote_state(state: &str) -> Option<RemoteSessionState> {
    match state {
        "pending" | "Pending" => Some(RemoteSessionState::Pending),
        "running" | "Running" => Some(RemoteSessionState::Running),
        "idle" | "Idle" => Some(RemoteSessionState::Idle),
        "requires_action" | "RequiresAction" => Some(RemoteSessionState::RequiresAction),
        "archived" | "Archived" => Some(RemoteSessionState::Archived),
        "expired" | "Expired" => Some(RemoteSessionState::Expired),
        "failed" | "Failed" => Some(RemoteSessionState::Failed),
        _ => None,
    }
}

#[derive(Debug, Serialize)]
struct CreateRemoteSessionRequestWire {
    environment_id: String,
    title: Option<String>,
    permission_mode: Option<String>,
    events: Vec<SessionEventWire>,
}

impl From<CreateRemoteSessionRequest> for CreateRemoteSessionRequestWire {
    fn from(value: CreateRemoteSessionRequest) -> Self {
        Self {
            environment_id: value.environment_id,
            title: value.title,
            permission_mode: value.permission_mode,
            events: value
                .events
                .into_iter()
                .map(|event| SessionEventWire {
                    event_type: event.event_type,
                    payload_json: event.payload_json,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
struct SessionEventWire {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(rename = "payload")]
    payload_json: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct RemoteSessionSummaryWire {
    session_id: String,
    title: Option<String>,
    environment_id: Option<String>,
    state: String,
}

#[derive(Debug, Serialize)]
struct PatchTitleWire {
    title: String,
}
