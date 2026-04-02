use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct CreateRemoteSessionRequest {
    pub environment_id: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct ArchivePolicy {
    pub idempotent: bool,
}

#[derive(Debug, Clone)]
pub struct RemoteSessionSummary {
    pub session_id: String,
    pub state: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RemoteSessionError {
    #[error("remote control is disabled by policy")]
    DisabledByPolicy,
    #[error("missing entitlement")]
    EntitlementDenied,
    #[error("invalid state transition")]
    InvalidStateTransition,
    #[error("auth unavailable")]
    AuthUnavailable,
    #[error("upstream error: {0}")]
    Upstream(String),
}

#[async_trait]
pub trait RemoteSessionService: Send + Sync {
    async fn create_session(
        &self,
        req: CreateRemoteSessionRequest,
    ) -> Result<RemoteSessionSummary, RemoteSessionError>;
    async fn fetch_session(
        &self,
        session_id: &str,
    ) -> Result<RemoteSessionSummary, RemoteSessionError>;
    async fn archive_session(
        &self,
        session_id: &str,
        policy: ArchivePolicy,
    ) -> Result<(), RemoteSessionError>;
    async fn update_title(&self, session_id: &str, title: &str) -> Result<(), RemoteSessionError>;
    async fn reconcile_resume(
        &self,
        session_id: &str,
    ) -> Result<RemoteSessionSummary, RemoteSessionError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerScope {
    SessionOnly,
    Durable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerOwner {
    MainAgent,
    Teammate(String),
    TeamLead,
}

#[derive(Debug, Clone)]
pub struct TriggerTask {
    pub id: String,
    pub cron: String,
    pub prompt: String,
    pub scope: TriggerScope,
    pub owner: TriggerOwner,
}

#[derive(Debug, thiserror::Error)]
pub enum TriggerError {
    #[error("gate disabled")]
    GateDisabled,
    #[error("invalid cron")]
    InvalidCron,
    #[error("ownership violation")]
    OwnershipViolation,
    #[error("durable tasks not allowed for teammates")]
    DurableNotAllowedForTeammate,
    #[error("unauthorized")]
    Unauthorized,
    #[error("upstream remote error")]
    UpstreamRemoteError,
    #[error("storage error")]
    StorageError,
}

#[async_trait]
pub trait TriggerSchedulerService: Send + Sync {
    async fn create(&self, task: TriggerTask) -> Result<TriggerTask, TriggerError>;
    async fn list(&self) -> Result<Vec<TriggerTask>, TriggerError>;
    async fn delete(&self, id: &str, actor: TriggerOwner) -> Result<(), TriggerError>;
}

#[async_trait]
pub trait RemoteTriggerDispatchService: Send + Sync {
    async fn dispatch(&self, payload: Value) -> Result<String, TriggerError>;
}

#[derive(Debug, Clone)]
pub struct SyncResult {
    pub files_pulled: usize,
    pub files_pushed: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum TeamMemError {
    #[error("auth unavailable")]
    AuthUnavailable,
    #[error("unauthorized")]
    Unauthorized,
    #[error("conflict exhausted")]
    ConflictExhausted,
    #[error("path validation failed")]
    PathValidationFailed,
    #[error("secret detected")]
    SecretDetected,
    #[error("file too large")]
    FileTooLarge,
    #[error("entry limit exceeded")]
    EntryLimitExceeded,
    #[error("storage write error")]
    StorageWriteError,
    #[error("upstream timeout")]
    UpstreamTimeout,
}

#[async_trait]
pub trait TeamMemorySyncService: Send + Sync {
    async fn pull(&self) -> Result<SyncResult, TeamMemError>;
    async fn push(&self) -> Result<SyncResult, TeamMemError>;
    async fn sync(&self) -> Result<SyncResult, TeamMemError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UltraplanPhase {
    Idle,
    Launching,
    Polling,
    AwaitingInput,
    Approved,
    Stopping,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct UltraplanSession {
    pub session_id: String,
    pub phase: UltraplanPhase,
}

#[derive(Debug, Clone, Copy)]
pub struct UltraplanPolicy {
    pub single_active_session: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum UltraplanError {
    #[error("disabled by policy")]
    DisabledByPolicy,
    #[error("already active")]
    AlreadyActive,
    #[error("launch failed")]
    LaunchFailed,
    #[error("poll timeout")]
    PollTimeout,
    #[error("approval failed")]
    ApprovalFailed,
    #[error("archive failed")]
    ArchiveFailed,
    #[error("transport error: {0}")]
    Transport(String),
}

#[async_trait]
pub trait UltraplanService: Send + Sync {
    async fn launch(
        &self,
        prompt: &str,
        policy: UltraplanPolicy,
    ) -> Result<UltraplanSession, UltraplanError>;
    async fn poll(&self, session_id: &str) -> Result<UltraplanPhase, UltraplanError>;
    async fn stop(&self, session_id: &str) -> Result<(), UltraplanError>;
    async fn archive_orphan(&self, session_id: &str) -> Result<(), UltraplanError>;
}

#[derive(Debug, Clone)]
pub struct AssistantModeContext;

#[derive(Debug, Clone)]
pub struct AssistantModeDecision;

#[derive(Debug, Clone)]
pub struct PromptComposeContext;

#[derive(Debug, Clone)]
pub struct PromptComposeResult;

#[derive(Debug, Clone)]
pub struct RouteInputContext;

#[derive(Debug, Clone)]
pub struct RouteDecision;

#[derive(Debug, thiserror::Error)]
pub enum KairosError {
    #[error("disabled by policy")]
    DisabledByPolicy,
    #[error("invalid mode state")]
    InvalidModeState,
    #[error("prompt compose failed: {0}")]
    PromptComposeFailed(String),
    #[error("route conflict")]
    RouteConflict,
}

pub trait AssistantModeService: Send + Sync {
    fn resolve_mode(&self, ctx: AssistantModeContext) -> AssistantModeDecision;
    fn build_prompt(&self, ctx: PromptComposeContext) -> PromptComposeResult;
    fn route_input(&self, ctx: RouteInputContext) -> RouteDecision;
}

#[derive(Debug, Clone)]
pub struct ModeResolutionInput;

#[derive(Debug, Clone)]
pub struct EffectiveMode;

#[derive(Debug, Clone)]
pub struct SessionMode;

#[derive(Debug, Clone)]
pub struct ModeReconcileResult;

#[derive(Debug, thiserror::Error)]
pub enum CoordinatorModeError {
    #[error("disabled by policy")]
    DisabledByPolicy,
    #[error("invalid mode transition")]
    InvalidModeTransition,
    #[error("session mode mismatch")]
    SessionModeMismatch,
}

pub trait ModeCoordinatorService: Send + Sync {
    fn resolve_effective_mode(&self, input: ModeResolutionInput) -> EffectiveMode;
    fn reconcile_on_resume(&self, session_mode: Option<SessionMode>) -> ModeReconcileResult;
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
    pub owner_scope: String,
}

#[derive(Debug, Clone)]
pub struct WorkerResultNotification {
    pub task_id: String,
    pub status: WorkerStatus,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct CoordinatorSummary {
    pub completed: usize,
    pub failed: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum OrchestrationError {
    #[error("disabled by mode")]
    DisabledByMode,
    #[error("invalid task spec")]
    InvalidTaskSpec,
    #[error("policy violation")]
    PolicyViolation,
    #[error("notification malformed")]
    NotificationMalformed,
    #[error("task not found")]
    TaskNotFound,
    #[error("spawn failed: {0}")]
    SpawnFailed(String),
    #[error("synthesis failed: {0}")]
    SynthesisFailed(String),
}

#[async_trait]
pub trait MultiAgentOrchestrator: Send + Sync {
    async fn spawn_parallel(
        &self,
        tasks: Vec<WorkerTaskSpec>,
    ) -> Result<Vec<String>, OrchestrationError>;
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
