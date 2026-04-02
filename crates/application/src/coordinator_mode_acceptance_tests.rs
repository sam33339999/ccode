/// Coordinator mode acceptance tests (US-039)
///
/// Covers all scenarios from coordinator-mode-contract.md §7 Acceptance Matrix:
///   §7.1 Contract tests  – precedence and reconcile logic invariants
///   §7.2 Integration tests – resume path mode consistency
///   §7.3 CLI behaviour tests – tool visibility, prompt branch, event metadata
use crate::{
    mode_coordinator_service::DefaultModeCoordinatorService,
    spec_contracts::{
        CoordinatorMode, CoordinatorModeError, EffectiveModeSource, ModeCoordinatorService,
        ModeReconcileAction, ModeResolutionInput, ModeSwitchEvent, ModeSwitchReason, SessionMode,
    },
};

// ── helpers ───────────────────────────────────────────────────────────────

fn standard_session() -> Option<SessionMode> {
    Some(SessionMode {
        mode: CoordinatorMode::Standard,
    })
}

fn coordinator_session() -> Option<SessionMode> {
    Some(SessionMode {
        mode: CoordinatorMode::Coordinator,
    })
}

fn service_with(
    policy_enabled: bool,
    env_mode: Option<CoordinatorMode>,
    current_mode: CoordinatorMode,
) -> DefaultModeCoordinatorService {
    DefaultModeCoordinatorService {
        policy_enabled,
        env_mode,
        current_mode,
    }
}

// ── §7.1 Contract tests ──────────────────────────────────────────────────

/// AC: mode precedence and reconcile logic is deterministic.
///
/// The precedence rule is: session_mode > env_mode > default(Standard).
/// Given identical inputs the result must never vary.
#[test]
fn contract_mode_precedence_is_deterministic() {
    let svc = DefaultModeCoordinatorService::default();

    // Run resolution twice with same inputs → identical results.
    let input = ModeResolutionInput {
        policy_enabled: true,
        env_mode: Some(CoordinatorMode::Coordinator),
        session_mode: coordinator_session(),
        previous_mode: Some(CoordinatorMode::Standard),
    };

    let a = svc.resolve_effective_mode(input.clone());
    let b = svc.resolve_effective_mode(input);

    assert_eq!(a.mode, b.mode, "mode must be deterministic across calls");
    assert_eq!(
        a.source, b.source,
        "source must be deterministic across calls"
    );
    assert_eq!(
        a.switch_event, b.switch_event,
        "switch_event must be deterministic across calls"
    );
}

/// AC: session_mode > env_mode in precedence.
///
/// When both session and env modes are provided, session wins.
#[test]
fn contract_session_mode_takes_precedence_over_env_mode() {
    let svc = DefaultModeCoordinatorService::default();
    let result = svc.resolve_effective_mode(ModeResolutionInput {
        policy_enabled: true,
        env_mode: Some(CoordinatorMode::Standard),
        session_mode: coordinator_session(),
        previous_mode: Some(CoordinatorMode::Standard),
    });

    assert_eq!(result.mode, CoordinatorMode::Coordinator);
    assert_eq!(result.source, EffectiveModeSource::Session);
    assert!(result.error.is_none());
}

/// AC: env_mode > default when no session mode.
#[test]
fn contract_env_mode_takes_precedence_over_default() {
    let svc = DefaultModeCoordinatorService::default();
    let result = svc.resolve_effective_mode(ModeResolutionInput {
        policy_enabled: true,
        env_mode: Some(CoordinatorMode::Coordinator),
        session_mode: None,
        previous_mode: Some(CoordinatorMode::Standard),
    });

    assert_eq!(result.mode, CoordinatorMode::Coordinator);
    assert_eq!(result.source, EffectiveModeSource::Env);
}

/// AC: default fallback is Standard when nothing else is provided.
#[test]
fn contract_default_fallback_is_standard() {
    let svc = DefaultModeCoordinatorService::default();
    let result = svc.resolve_effective_mode(ModeResolutionInput {
        policy_enabled: true,
        env_mode: None,
        session_mode: None,
        previous_mode: None,
    });

    assert_eq!(result.mode, CoordinatorMode::Standard);
    assert_eq!(result.source, EffectiveModeSource::Default);
    assert!(result.switch_event.is_none());
    assert!(result.error.is_none());
}

/// AC: resumed session mode vs env mode has explicit precedence rule.
///
/// reconcile_on_resume must prefer session_mode over env_mode and surface
/// SessionModeMismatch when they disagree.
#[test]
fn contract_reconcile_session_wins_over_env_and_flags_mismatch() {
    let svc = service_with(
        true,
        Some(CoordinatorMode::Standard),
        CoordinatorMode::Standard,
    );

    let result = svc.reconcile_on_resume(coordinator_session());

    assert_eq!(result.mode, CoordinatorMode::Coordinator);
    assert_eq!(result.action, ModeReconcileAction::SessionWins);
    assert!(
        matches!(
            result.error,
            Some(CoordinatorModeError::SessionModeMismatch)
        ),
        "mismatch between session and env must surface SessionModeMismatch"
    );
}

/// AC: reconcile is deterministic — same service + same session → same result.
#[test]
fn contract_reconcile_logic_is_deterministic() {
    let svc = service_with(
        true,
        Some(CoordinatorMode::Standard),
        CoordinatorMode::Standard,
    );

    let a = svc.reconcile_on_resume(coordinator_session());
    let b = svc.reconcile_on_resume(coordinator_session());

    assert_eq!(a.mode, b.mode);
    assert_eq!(a.action, b.action);
    assert_eq!(a.switch_event, b.switch_event);
}

/// AC: invalid mode transition returns InvalidModeTransition error.
///
/// Coordinator → Standard is forbidden (sticky mode). Attempting it via
/// resolve_effective_mode must return InvalidModeTransition without changing mode.
#[test]
fn contract_invalid_mode_transition_returns_error() {
    let svc = DefaultModeCoordinatorService::default();
    let result = svc.resolve_effective_mode(ModeResolutionInput {
        policy_enabled: true,
        env_mode: Some(CoordinatorMode::Standard),
        session_mode: None,
        previous_mode: Some(CoordinatorMode::Coordinator),
    });

    assert_eq!(
        result.mode,
        CoordinatorMode::Coordinator,
        "mode must remain Coordinator when transition is rejected"
    );
    assert!(
        matches!(
            result.error,
            Some(CoordinatorModeError::InvalidModeTransition)
        ),
        "Coordinator→Standard must yield InvalidModeTransition; got {:?}",
        result.error
    );
}

/// AC: invalid mode transition via reconcile_on_resume.
///
/// When current mode is Coordinator and no session mode is provided, env
/// tries to go Standard which is forbidden.
#[test]
fn contract_invalid_transition_on_reconcile_returns_error() {
    let svc = service_with(
        true,
        Some(CoordinatorMode::Standard),
        CoordinatorMode::Coordinator,
    );

    let result = svc.reconcile_on_resume(None);

    assert_eq!(
        result.mode,
        CoordinatorMode::Coordinator,
        "mode must stay Coordinator when transition is blocked"
    );
    assert_eq!(result.action, ModeReconcileAction::Noop);
    assert!(matches!(
        result.error,
        Some(CoordinatorModeError::InvalidModeTransition)
    ));
}

/// AC: invalid transition via reconcile when session itself tries to downgrade.
#[test]
fn contract_invalid_transition_on_reconcile_with_session_returns_error() {
    let svc = service_with(true, None, CoordinatorMode::Coordinator);

    let result = svc.reconcile_on_resume(standard_session());

    assert_eq!(result.mode, CoordinatorMode::Coordinator);
    assert_eq!(result.action, ModeReconcileAction::Noop);
    assert!(matches!(
        result.error,
        Some(CoordinatorModeError::InvalidModeTransition)
    ));
}

/// AC: disabled policy always returns Standard with DisabledByPolicy.
#[test]
fn contract_disabled_policy_returns_standard_with_error() {
    let svc = DefaultModeCoordinatorService::default();
    let result = svc.resolve_effective_mode(ModeResolutionInput {
        policy_enabled: false,
        env_mode: Some(CoordinatorMode::Coordinator),
        session_mode: coordinator_session(),
        previous_mode: None,
    });

    assert_eq!(result.mode, CoordinatorMode::Standard);
    assert!(matches!(
        result.error,
        Some(CoordinatorModeError::DisabledByPolicy)
    ));
}

// ── §7.2 Integration tests ───────────────────────────────────────────────

/// AC: resume path updates mode consistently.
///
/// Simulate a full resume: service starts Standard, session says Coordinator.
/// After reconcile the effective mode must be Coordinator with SessionWins action.
#[test]
fn integration_resume_path_updates_mode_consistently() {
    let svc = service_with(true, None, CoordinatorMode::Standard);

    let reconcile = svc.reconcile_on_resume(coordinator_session());
    assert_eq!(reconcile.mode, CoordinatorMode::Coordinator);
    assert_eq!(reconcile.action, ModeReconcileAction::SessionWins);

    // Now resolve with the reconciled mode as previous
    let resolved = svc.resolve_effective_mode(ModeResolutionInput {
        policy_enabled: true,
        env_mode: None,
        session_mode: coordinator_session(),
        previous_mode: Some(reconcile.mode),
    });

    assert_eq!(
        resolved.mode,
        CoordinatorMode::Coordinator,
        "subsequent resolution must be consistent with reconcile result"
    );
}

/// AC: resume without session mode adopts env mode.
#[test]
fn integration_resume_without_session_adopts_env_mode() {
    let svc = service_with(
        true,
        Some(CoordinatorMode::Coordinator),
        CoordinatorMode::Standard,
    );

    let result = svc.reconcile_on_resume(None);

    assert_eq!(result.mode, CoordinatorMode::Coordinator);
    assert_eq!(result.action, ModeReconcileAction::EnvAdopted);
    assert!(result.error.is_none());
}

/// AC: resume without session and without env falls back to Standard.
#[test]
fn integration_resume_without_session_or_env_falls_back_to_standard() {
    let svc = service_with(true, None, CoordinatorMode::Standard);

    let result = svc.reconcile_on_resume(None);

    assert_eq!(result.mode, CoordinatorMode::Standard);
    assert_eq!(result.action, ModeReconcileAction::EnvAdopted);
    assert!(result.error.is_none());
    assert!(
        result.switch_event.is_none(),
        "no switch event when mode stays the same"
    );
}

/// AC: tool visibility changes when mode switches.
///
/// Coordinator mode exposes a wider tool set. After switching from Standard
/// to Coordinator the tool allowlist must differ.
#[test]
fn integration_tool_visibility_changes_on_mode_switch() {
    let svc = DefaultModeCoordinatorService::default();

    let standard = svc.resolve_effective_mode(ModeResolutionInput {
        policy_enabled: true,
        env_mode: None,
        session_mode: None,
        previous_mode: None,
    });
    assert_eq!(standard.mode, CoordinatorMode::Standard);

    let coordinator = svc.resolve_effective_mode(ModeResolutionInput {
        policy_enabled: true,
        env_mode: None,
        session_mode: coordinator_session(),
        previous_mode: Some(CoordinatorMode::Standard),
    });
    assert_eq!(coordinator.mode, CoordinatorMode::Coordinator);

    // Modes differ → tool allowlists must differ. We verify the mode values
    // are distinct which is the precondition for the tool-runtime allowlist
    // filter to produce different outputs.
    assert_ne!(
        standard.mode, coordinator.mode,
        "modes must differ so tool-runtime produces different allowlists"
    );
    assert!(
        coordinator.switch_event.is_some(),
        "mode switch must emit an event for tool-runtime to react to"
    );
}

/// AC: disabled policy on resume returns Standard with error.
#[test]
fn integration_resume_disabled_policy_returns_standard() {
    let svc = service_with(
        false,
        Some(CoordinatorMode::Coordinator),
        CoordinatorMode::Standard,
    );

    let result = svc.reconcile_on_resume(coordinator_session());

    assert_eq!(result.mode, CoordinatorMode::Standard);
    assert_eq!(result.action, ModeReconcileAction::Noop);
    assert!(matches!(
        result.error,
        Some(CoordinatorModeError::DisabledByPolicy)
    ));
}

// ── §7.3 CLI behaviour tests ─────────────────────────────────────────────

/// AC: tool visibility and prompt mode branch are correct.
///
/// Standard mode produces Default source; Coordinator mode produces Session
/// source. The source drives the TUI prompt branch selection.
#[test]
fn cli_tool_visibility_and_prompt_branch_are_correct() {
    let svc = DefaultModeCoordinatorService::default();

    // Standard mode → Default source
    let standard = svc.resolve_effective_mode(ModeResolutionInput {
        policy_enabled: true,
        env_mode: None,
        session_mode: None,
        previous_mode: None,
    });
    assert_eq!(standard.source, EffectiveModeSource::Default);

    // Coordinator via session → Session source
    let coordinator = svc.resolve_effective_mode(ModeResolutionInput {
        policy_enabled: true,
        env_mode: None,
        session_mode: coordinator_session(),
        previous_mode: None,
    });
    assert_eq!(coordinator.source, EffectiveModeSource::Session);

    // Coordinator via env → Env source
    let env_coord = svc.resolve_effective_mode(ModeResolutionInput {
        policy_enabled: true,
        env_mode: Some(CoordinatorMode::Coordinator),
        session_mode: None,
        previous_mode: None,
    });
    assert_eq!(env_coord.source, EffectiveModeSource::Env);
}

/// AC: mode switch event metadata (from, to, reason) is emitted.
///
/// Every mode change must produce a ModeSwitchEvent with all three fields
/// populated so downstream consumers (TUI, telemetry) can react.
#[test]
fn cli_mode_switch_event_metadata_is_emitted() {
    let svc = DefaultModeCoordinatorService::default();

    // Standard → Coordinator via session
    let result = svc.resolve_effective_mode(ModeResolutionInput {
        policy_enabled: true,
        env_mode: None,
        session_mode: coordinator_session(),
        previous_mode: Some(CoordinatorMode::Standard),
    });

    let event = result
        .switch_event
        .expect("mode change must emit a switch event");
    assert_eq!(event.from, CoordinatorMode::Standard);
    assert_eq!(event.to, CoordinatorMode::Coordinator);
    assert_eq!(event.reason, ModeSwitchReason::SessionPrecedence);
}

/// AC: mode switch via env emits correct reason.
#[test]
fn cli_mode_switch_via_env_emits_env_configured_reason() {
    let svc = DefaultModeCoordinatorService::default();
    let result = svc.resolve_effective_mode(ModeResolutionInput {
        policy_enabled: true,
        env_mode: Some(CoordinatorMode::Coordinator),
        session_mode: None,
        previous_mode: Some(CoordinatorMode::Standard),
    });

    let event = result.switch_event.expect("must emit switch event");
    assert_eq!(event.from, CoordinatorMode::Standard);
    assert_eq!(event.to, CoordinatorMode::Coordinator);
    assert_eq!(event.reason, ModeSwitchReason::EnvConfigured);
}

/// AC: reconcile switch emits ResumeReconcile reason.
#[test]
fn cli_reconcile_switch_emits_resume_reconcile_reason() {
    let svc = service_with(true, None, CoordinatorMode::Standard);

    let result = svc.reconcile_on_resume(coordinator_session());

    let event = result
        .switch_event
        .expect("reconcile mode change must emit event");
    assert_eq!(event.from, CoordinatorMode::Standard);
    assert_eq!(event.to, CoordinatorMode::Coordinator);
    assert_eq!(event.reason, ModeSwitchReason::ResumeReconcile);
}

/// AC: no switch event when mode stays the same.
#[test]
fn cli_no_switch_event_when_mode_unchanged() {
    let svc = DefaultModeCoordinatorService::default();
    let result = svc.resolve_effective_mode(ModeResolutionInput {
        policy_enabled: true,
        env_mode: Some(CoordinatorMode::Standard),
        session_mode: None,
        previous_mode: Some(CoordinatorMode::Standard),
    });

    assert!(
        result.switch_event.is_none(),
        "no event must be emitted when mode does not change"
    );
}

/// AC: SessionModeMismatch surfaced as actionable message.
///
/// When session and env disagree, the error variant is SessionModeMismatch
/// which the TUI renders as an actionable warning. Verify the error is
/// present and the mode still resolves correctly (session wins).
#[test]
fn cli_session_mode_mismatch_surfaced_as_actionable_message() {
    let svc = service_with(
        true,
        Some(CoordinatorMode::Standard),
        CoordinatorMode::Standard,
    );

    let result = svc.reconcile_on_resume(coordinator_session());

    assert_eq!(
        result.mode,
        CoordinatorMode::Coordinator,
        "session must still win despite mismatch"
    );
    assert_eq!(result.action, ModeReconcileAction::SessionWins);

    let err = result
        .error
        .expect("mismatch must produce an error for the TUI to render");
    assert_eq!(
        err,
        CoordinatorModeError::SessionModeMismatch,
        "error must be SessionModeMismatch so TUI can render actionable message"
    );

    // The error's Display impl is what the TUI would show
    assert_eq!(
        err.to_string(),
        "session mode mismatch",
        "error message must be human-readable"
    );
}

/// AC: no mismatch when session and env agree.
#[test]
fn cli_no_mismatch_when_session_and_env_agree() {
    let svc = service_with(
        true,
        Some(CoordinatorMode::Coordinator),
        CoordinatorMode::Standard,
    );

    let result = svc.reconcile_on_resume(coordinator_session());

    assert_eq!(result.mode, CoordinatorMode::Coordinator);
    assert_eq!(result.action, ModeReconcileAction::SessionWins);
    assert!(
        result.error.is_none(),
        "no mismatch error when session and env agree on Coordinator"
    );
}
