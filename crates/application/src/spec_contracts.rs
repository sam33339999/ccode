use async_trait::async_trait;
use ccode_domain::{
    assistant_mode::{AssistantMode, ModeSwitchTrigger},
    event::DomainEvent,
    session::SessionId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct CreateRemoteSessionRequest {
    pub environment_id: String,
    pub title: Option<String>,
    pub permission_mode: Option<String>,
    pub events: Vec<SessionEvent>,
}

#[derive(Debug, Clone)]
pub struct SessionEvent {
    pub event_type: String,
    pub payload_json: Value,
}

#[derive(Debug, Clone, Copy)]
pub struct ArchivePolicy {
    pub timeout: Duration,
    pub idempotent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteSessionState {
    Pending,
    Running,
    Idle,
    RequiresAction,
    Archived,
    Expired,
    Failed,
}

#[derive(Debug, Clone)]
pub struct RemoteSessionSummary {
    pub session_id: String,
    pub title: Option<String>,
    pub environment_id: Option<String>,
    pub state: RemoteSessionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveResult {
    Archived,
    AlreadyArchived,
}

#[derive(Debug, thiserror::Error)]
pub enum CcrClientError {
    #[error("http error: {0}")]
    Http(String),
    #[error("timeout")]
    Timeout,
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("invalid payload")]
    InvalidPayload,
}

#[derive(Debug, thiserror::Error)]
pub enum RemoteSessionError {
    #[error("remote control is disabled by policy")]
    DisabledByPolicy,
    #[error("missing entitlement")]
    EntitlementDenied,
    #[error("invalid state transition")]
    InvalidStateTransition,
    #[error("session not found")]
    NotFound,
    #[error("session expired")]
    Expired,
    #[error("auth unavailable")]
    AuthUnavailable,
    #[error("upstream error: {0}")]
    Upstream(String),
}

#[async_trait]
pub trait CcrClient: Send + Sync {
    async fn create(
        &self,
        req: CreateRemoteSessionRequest,
    ) -> Result<RemoteSessionSummary, CcrClientError>;
    async fn get(&self, session_id: &str) -> Result<RemoteSessionSummary, CcrClientError>;
    async fn archive(&self, session_id: &str) -> Result<ArchiveResult, CcrClientError>;
    async fn patch_title(&self, session_id: &str, title: &str) -> Result<(), CcrClientError>;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerScope {
    SessionOnly,
    Durable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerOwner {
    MainAgent,
    Teammate(String),
    TeamLead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerTask {
    pub id: String,
    pub cron: String,
    pub prompt: String,
    pub scope: TriggerScope,
    pub owner: TriggerOwner,
    pub durable_intent: bool,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityPolicy {
    pub allow_assistant_modes: bool,
    pub allow_brief_mode: bool,
    pub allow_channels_mode: bool,
    pub blocked_tools: Vec<String>,
    pub brief_blocked_tools: Vec<String>,
}

impl Default for CapabilityPolicy {
    fn default() -> Self {
        Self {
            allow_assistant_modes: true,
            allow_brief_mode: true,
            allow_channels_mode: true,
            blocked_tools: Vec::new(),
            brief_blocked_tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolVisibilityMode {
    All,
    HideRestricted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantModeContext {
    pub session_id: SessionId,
    pub configured_mode: AssistantMode,
    pub session_mode: Option<AssistantMode>,
    pub policy_enabled: bool,
    pub available_tools: Vec<String>,
    pub capability_policy: CapabilityPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedModeSource {
    Config,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KairosTelemetryTags {
    pub mode: AssistantMode,
    pub mode_source: ModeSwitchTrigger,
    pub kairos_active: bool,
    pub brief_active: bool,
    pub channels_active: bool,
    pub prompt_layer_count: usize,
}

#[derive(Debug, Clone)]
pub struct AssistantModeDecision {
    pub effective_mode: AssistantMode,
    pub source: ResolvedModeSource,
    pub switch_event: Option<DomainEvent>,
    pub visible_tools: Vec<String>,
    pub telemetry_tags: KairosTelemetryTags,
    pub error: Option<KairosError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptPrecedenceLayer {
    Base,
    ModeDefault,
    ModeOverride,
    Policy,
    Runtime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptComposeContext {
    pub mode: AssistantMode,
    pub policy_enabled: bool,
    pub capability_policy: CapabilityPolicy,
    pub base_prompt: Option<String>,
    pub mode_prompt_override: Option<String>,
    pub policy_prompt: Option<String>,
    pub runtime_prompt: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PromptComposeResult {
    pub system_prompt: String,
    pub precedence: Vec<PromptPrecedenceLayer>,
    pub telemetry_tags: KairosTelemetryTags,
    pub error: Option<KairosError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteInputContext {
    pub session_id: SessionId,
    pub current_mode: AssistantMode,
    pub policy_enabled: bool,
    pub capability_policy: CapabilityPolicy,
    pub raw_input: String,
    pub explicit_mode: Option<AssistantMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteSource {
    SlashCommand,
    ExplicitOverride,
    Noop,
    Conflict,
}

#[derive(Debug, Clone)]
pub struct RouteDecision {
    pub source: RouteSource,
    pub next_mode: AssistantMode,
    pub passthrough_input: String,
    pub command_consumed: bool,
    pub switch_event: Option<DomainEvent>,
    pub telemetry_tags: KairosTelemetryTags,
    pub error: Option<KairosError>,
}

#[derive(Debug, Clone, thiserror::Error)]
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
pub struct ModeResolutionInput {
    pub policy_enabled: bool,
    pub env_mode: Option<CoordinatorMode>,
    pub session_mode: Option<SessionMode>,
    pub previous_mode: Option<CoordinatorMode>,
}

impl ModeResolutionInput {
    pub fn from_env(
        policy_enabled: bool,
        session_mode: Option<SessionMode>,
        previous_mode: Option<CoordinatorMode>,
    ) -> Self {
        Self {
            policy_enabled,
            env_mode: std::env::var(CLAUDE_CODE_COORDINATOR_MODE)
                .ok()
                .as_deref()
                .and_then(CoordinatorMode::parse),
            session_mode,
            previous_mode,
        }
    }
}

pub const CLAUDE_CODE_COORDINATOR_MODE: &str = "CLAUDE_CODE_COORDINATOR_MODE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinatorMode {
    Standard,
    Coordinator,
}

impl CoordinatorMode {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "standard" | "off" | "false" | "disabled" => Some(Self::Standard),
            "coordinator" | "on" | "true" | "enabled" => Some(Self::Coordinator),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveModeSource {
    Default,
    Env,
    Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionMode {
    pub mode: CoordinatorMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeSwitchReason {
    SessionPrecedence,
    EnvConfigured,
    DefaultFallback,
    ResumeReconcile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeSwitchEvent {
    pub from: CoordinatorMode,
    pub to: CoordinatorMode,
    pub reason: ModeSwitchReason,
}

#[derive(Debug, Clone)]
pub struct EffectiveMode {
    pub mode: CoordinatorMode,
    pub source: EffectiveModeSource,
    pub switch_event: Option<ModeSwitchEvent>,
    pub error: Option<CoordinatorModeError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeReconcileAction {
    Noop,
    SessionWins,
    EnvAdopted,
}

#[derive(Debug, Clone)]
pub struct ModeReconcileResult {
    pub mode: CoordinatorMode,
    pub action: ModeReconcileAction,
    pub switch_event: Option<ModeSwitchEvent>,
    pub error: Option<CoordinatorModeError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
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
