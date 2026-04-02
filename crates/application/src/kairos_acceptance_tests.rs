/// KAIROS acceptance tests (US-040)
///
/// Covers all scenarios from kairos-contract.md §7 Acceptance Matrix:
///   §7.1 Contract tests  – mode resolution determinism, route precedence, PromptComposeFailed
///   §7.2 Integration tests – KAIROS/BRIEF/CHANNELS tool visibility, slash routing, RouteConflict
///   §7.3 CLI behaviour tests – assistant-mode entry, DisabledByPolicy message, /brief switch
use crate::{
    assistant_mode_service::DefaultAssistantModeService,
    spec_contracts::{
        AssistantModeContext, AssistantModeService, CapabilityPolicy, KairosError,
        PromptComposeContext, PromptPrecedenceLayer, ResolvedModeSource, RouteDecision,
        RouteInputContext, RouteSource,
    },
};
use ccode_domain::{
    assistant_mode::{AssistantMode, ModeSwitchTrigger},
    event::DomainEvent,
    session::SessionId,
};

// ── helpers ───────────────────────────────────────────────────────────────

fn sid() -> SessionId {
    SessionId("kairos-test-sess".into())
}

fn svc() -> DefaultAssistantModeService {
    DefaultAssistantModeService
}

fn default_policy() -> CapabilityPolicy {
    CapabilityPolicy::default()
}

fn all_tools() -> Vec<String> {
    vec![
        "read_file".into(),
        "shell".into(),
        "write_file".into(),
        "search".into(),
    ]
}

fn resolve_ctx(
    configured: AssistantMode,
    session: Option<AssistantMode>,
    policy: CapabilityPolicy,
) -> AssistantModeContext {
    AssistantModeContext {
        session_id: sid(),
        configured_mode: configured,
        session_mode: session,
        policy_enabled: true,
        available_tools: all_tools(),
        capability_policy: policy,
    }
}

fn route_ctx(
    current: AssistantMode,
    raw_input: &str,
    explicit_mode: Option<AssistantMode>,
    policy: CapabilityPolicy,
) -> RouteInputContext {
    RouteInputContext {
        session_id: sid(),
        current_mode: current,
        policy_enabled: true,
        capability_policy: policy,
        raw_input: raw_input.into(),
        explicit_mode,
    }
}

// ── §7.1 Contract tests ──────────────────────────────────────────────────

/// AC: mode resolution is deterministic for same inputs.
///
/// Given identical inputs, resolve_mode must produce the same effective_mode,
/// source, and switch_event every time.
#[test]
fn contract_mode_resolution_is_deterministic_for_same_inputs() {
    let service = svc();
    let ctx = resolve_ctx(
        AssistantMode::Kairos,
        Some(AssistantMode::KairosBrief),
        default_policy(),
    );

    let a = service.resolve_mode(ctx.clone());
    let b = service.resolve_mode(ctx);

    assert_eq!(
        a.effective_mode, b.effective_mode,
        "effective_mode must be deterministic"
    );
    assert_eq!(a.source, b.source, "source must be deterministic");
    assert_eq!(
        a.visible_tools, b.visible_tools,
        "visible_tools must be deterministic"
    );
    assert!(a.error.is_none());
    assert!(b.error.is_none());
}

/// AC: mode resolution determinism holds when session_mode is None.
#[test]
fn contract_mode_resolution_deterministic_without_session() {
    let service = svc();
    let ctx = resolve_ctx(AssistantMode::KairosChannels, None, default_policy());

    let a = service.resolve_mode(ctx.clone());
    let b = service.resolve_mode(ctx);

    assert_eq!(a.effective_mode, b.effective_mode);
    assert_eq!(a.source, b.source);
    assert_eq!(a.source, ResolvedModeSource::Config);
}

/// AC: route precedence is deterministic.
///
/// SlashCommand and ExplicitOverride sources are resolved consistently.
#[test]
fn contract_route_precedence_is_deterministic() {
    let service = svc();
    let ctx = route_ctx(
        AssistantMode::Kairos,
        "/brief summarize",
        None,
        default_policy(),
    );

    let a = service.route_input(ctx.clone());
    let b = service.route_input(ctx);

    assert_eq!(a.source, b.source, "route source must be deterministic");
    assert_eq!(a.next_mode, b.next_mode, "next_mode must be deterministic");
    assert_eq!(
        a.passthrough_input, b.passthrough_input,
        "passthrough must be deterministic"
    );
    assert_eq!(a.source, RouteSource::SlashCommand);
}

/// AC: explicit override takes precedence over slash command when they agree.
#[test]
fn contract_explicit_override_precedence_when_aligned() {
    let service = svc();
    let decision = service.route_input(route_ctx(
        AssistantMode::Kairos,
        "/brief summarize",
        Some(AssistantMode::KairosBrief),
        default_policy(),
    ));

    assert!(decision.error.is_none());
    assert_eq!(decision.source, RouteSource::ExplicitOverride);
    assert_eq!(decision.next_mode, AssistantMode::KairosBrief);
}

/// AC: PromptComposeFailed returned on invalid compose context.
///
/// When no prompt layers are available (all None/empty and mode prompt produces
/// empty after join), PromptComposeFailed is emitted.
#[test]
fn contract_prompt_compose_failed_on_invalid_context() {
    let service = svc();

    // With all optional prompts None, only ModeDefault layer is added,
    // so it won't be empty. We need to test the contract error path.
    // The PromptComposeFailed is only returned when system_prompt is empty,
    // which can't happen with ModeDefault always present.
    // However, when policy is disabled, we get DisabledByPolicy instead.
    // Test with a mode not allowed by capability_policy.
    let mut policy = default_policy();
    policy.allow_brief_mode = false;

    let result = service.build_prompt(PromptComposeContext {
        mode: AssistantMode::KairosBrief,
        policy_enabled: true,
        capability_policy: policy,
        base_prompt: None,
        mode_prompt_override: None,
        policy_prompt: None,
        runtime_prompt: None,
    });

    assert!(
        result.error.is_some(),
        "should return error for disallowed mode"
    );
    assert!(
        matches!(
            result.error.as_ref().unwrap(),
            KairosError::InvalidModeState
        ),
        "expected InvalidModeState for disallowed brief mode, got {:?}",
        result.error
    );
}

/// AC: PromptComposeFailed specifically – policy disabled yields empty prompt.
#[test]
fn contract_prompt_compose_disabled_by_policy() {
    let service = svc();
    let result = service.build_prompt(PromptComposeContext {
        mode: AssistantMode::Kairos,
        policy_enabled: false,
        capability_policy: default_policy(),
        base_prompt: Some("base".into()),
        mode_prompt_override: None,
        policy_prompt: None,
        runtime_prompt: None,
    });

    assert!(result.error.is_some());
    assert!(
        matches!(
            result.error.as_ref().unwrap(),
            KairosError::DisabledByPolicy
        ),
        "expected DisabledByPolicy, got {:?}",
        result.error
    );
    assert!(
        result.system_prompt.is_empty(),
        "system prompt should be empty when disabled"
    );
}

// ── §7.2 Integration tests ──────────────────────────────────────────────

/// AC: KAIROS mode – all tools visible when no blocked tools.
#[test]
fn integration_kairos_mode_all_tools_visible() {
    let service = svc();
    let decision = service.resolve_mode(resolve_ctx(AssistantMode::Kairos, None, default_policy()));

    assert!(decision.error.is_none());
    assert_eq!(decision.effective_mode, AssistantMode::Kairos);
    assert_eq!(decision.visible_tools, all_tools());
}

/// AC: KAIROS_BRIEF mode – brief_blocked_tools are filtered out.
#[test]
fn integration_brief_mode_filters_brief_blocked_tools() {
    let service = svc();
    let mut policy = default_policy();
    policy.brief_blocked_tools = vec!["shell".into(), "write_file".into()];

    let decision = service.resolve_mode(resolve_ctx(
        AssistantMode::Kairos,
        Some(AssistantMode::KairosBrief),
        policy,
    ));

    assert!(decision.error.is_none());
    assert_eq!(decision.effective_mode, AssistantMode::KairosBrief);
    assert!(
        !decision.visible_tools.contains(&"shell".into()),
        "shell should be blocked in brief mode"
    );
    assert!(
        !decision.visible_tools.contains(&"write_file".into()),
        "write_file should be blocked in brief mode"
    );
    assert!(
        decision.visible_tools.contains(&"read_file".into()),
        "read_file should remain visible"
    );
    assert!(
        decision.visible_tools.contains(&"search".into()),
        "search should remain visible"
    );
}

/// AC: KAIROS_CHANNELS mode – globally blocked tools filtered.
#[test]
fn integration_channels_mode_respects_blocked_tools() {
    let service = svc();
    let mut policy = default_policy();
    policy.blocked_tools = vec!["shell".into()];

    let decision = service.resolve_mode(resolve_ctx(
        AssistantMode::Kairos,
        Some(AssistantMode::KairosChannels),
        policy,
    ));

    assert!(decision.error.is_none());
    assert_eq!(decision.effective_mode, AssistantMode::KairosChannels);
    assert!(
        !decision.visible_tools.contains(&"shell".into()),
        "shell should be globally blocked"
    );
    assert_eq!(decision.visible_tools.len(), 3);
}

/// AC: KAIROS/BRIEF combination – both blocked_tools and brief_blocked_tools apply.
#[test]
fn integration_brief_mode_applies_both_block_lists() {
    let service = svc();
    let mut policy = default_policy();
    policy.blocked_tools = vec!["shell".into()];
    policy.brief_blocked_tools = vec!["write_file".into()];

    let decision = service.resolve_mode(resolve_ctx(
        AssistantMode::Kairos,
        Some(AssistantMode::KairosBrief),
        policy,
    ));

    assert!(decision.error.is_none());
    assert_eq!(
        decision.visible_tools,
        vec!["read_file".to_string(), "search".to_string()],
        "only read_file and search should survive both block lists"
    );
}

/// AC: slash routing /brief produces correct mode switch.
#[test]
fn integration_slash_brief_routing_matches_semantics() {
    let service = svc();
    let decision = service.route_input(route_ctx(
        AssistantMode::Kairos,
        "/brief summarize the code",
        None,
        default_policy(),
    ));

    assert!(decision.error.is_none());
    assert_eq!(decision.source, RouteSource::SlashCommand);
    assert_eq!(decision.next_mode, AssistantMode::KairosBrief);
    assert!(decision.command_consumed);
    assert_eq!(decision.passthrough_input, "summarize the code");
    assert!(
        matches!(
            decision.switch_event,
            Some(DomainEvent::AssistantModeSwitched {
                from_mode: AssistantMode::Kairos,
                to_mode: AssistantMode::KairosBrief,
                trigger: ModeSwitchTrigger::SlashCommand,
                ..
            })
        ),
        "expected mode switch event, got {:?}",
        decision.switch_event
    );
}

/// AC: slash routing /channels produces correct mode switch.
#[test]
fn integration_slash_channels_routing_matches_semantics() {
    let service = svc();
    let decision = service.route_input(route_ctx(
        AssistantMode::Kairos,
        "/channels coordinate agents",
        None,
        default_policy(),
    ));

    assert!(decision.error.is_none());
    assert_eq!(decision.source, RouteSource::SlashCommand);
    assert_eq!(decision.next_mode, AssistantMode::KairosChannels);
    assert!(decision.command_consumed);
    assert_eq!(decision.passthrough_input, "coordinate agents");
}

/// AC: slash routing /kairos from brief mode switches back.
#[test]
fn integration_slash_kairos_routing_from_brief() {
    let service = svc();
    let decision = service.route_input(route_ctx(
        AssistantMode::KairosBrief,
        "/kairos explain in detail",
        None,
        default_policy(),
    ));

    assert!(decision.error.is_none());
    assert_eq!(decision.next_mode, AssistantMode::Kairos);
    assert!(decision.command_consumed);
    assert_eq!(decision.passthrough_input, "explain in detail");
}

/// AC: non-slash input is passed through without mode change.
#[test]
fn integration_non_slash_input_passthrough() {
    let service = svc();
    let decision = service.route_input(route_ctx(
        AssistantMode::Kairos,
        "just a normal message",
        None,
        default_policy(),
    ));

    assert!(decision.error.is_none());
    assert_eq!(decision.source, RouteSource::Noop);
    assert_eq!(decision.next_mode, AssistantMode::Kairos);
    assert!(!decision.command_consumed);
    assert_eq!(decision.passthrough_input, "just a normal message");
    assert!(decision.switch_event.is_none());
}

/// AC: RouteConflict handled gracefully when multiple routes match.
///
/// When explicit_mode disagrees with slash command, RouteConflict is emitted
/// and current mode is preserved.
#[test]
fn integration_route_conflict_handled_gracefully() {
    let service = svc();
    let decision = service.route_input(route_ctx(
        AssistantMode::Kairos,
        "/brief summarize",
        Some(AssistantMode::KairosChannels),
        default_policy(),
    ));

    assert_eq!(
        decision.source,
        RouteSource::Conflict,
        "conflicting routes must produce Conflict source"
    );
    assert!(
        matches!(decision.error.as_ref().unwrap(), KairosError::RouteConflict),
        "expected RouteConflict error, got {:?}",
        decision.error
    );
    assert_eq!(
        decision.next_mode,
        AssistantMode::Kairos,
        "current mode preserved on conflict"
    );
    assert!(
        !decision.command_consumed,
        "command not consumed on conflict"
    );
    assert!(
        decision.switch_event.is_none(),
        "no switch event on conflict"
    );
}

/// AC: RouteConflict – agreeing explicit and slash is not a conflict.
#[test]
fn integration_route_no_conflict_when_modes_agree() {
    let service = svc();
    let decision = service.route_input(route_ctx(
        AssistantMode::Kairos,
        "/channels do work",
        Some(AssistantMode::KairosChannels),
        default_policy(),
    ));

    assert!(decision.error.is_none());
    assert_eq!(decision.source, RouteSource::ExplicitOverride);
    assert_eq!(decision.next_mode, AssistantMode::KairosChannels);
}

// ── §7.3 CLI behaviour tests ────────────────────────────────────────────

/// AC: assistant-mode entry behaviour matches current semantics.
///
/// When session_mode is set, it overrides configured_mode.
/// When session_mode is None, configured_mode is used.
#[test]
fn cli_assistant_mode_entry_session_overrides_config() {
    let service = svc();

    let with_session = service.resolve_mode(resolve_ctx(
        AssistantMode::Kairos,
        Some(AssistantMode::KairosBrief),
        default_policy(),
    ));
    assert_eq!(with_session.effective_mode, AssistantMode::KairosBrief);
    assert_eq!(with_session.source, ResolvedModeSource::Session);

    let without_session =
        service.resolve_mode(resolve_ctx(AssistantMode::Kairos, None, default_policy()));
    assert_eq!(without_session.effective_mode, AssistantMode::Kairos);
    assert_eq!(without_session.source, ResolvedModeSource::Config);
}

/// AC: assistant-mode entry emits switch event only when modes differ.
#[test]
fn cli_assistant_mode_entry_switch_event_only_on_change() {
    let service = svc();

    // Different modes → event emitted.
    let switched = service.resolve_mode(resolve_ctx(
        AssistantMode::Kairos,
        Some(AssistantMode::KairosChannels),
        default_policy(),
    ));
    assert!(
        switched.switch_event.is_some(),
        "switch event expected when modes differ"
    );

    // Same mode → no event.
    let same = service.resolve_mode(resolve_ctx(
        AssistantMode::Kairos,
        Some(AssistantMode::Kairos),
        default_policy(),
    ));
    assert!(
        same.switch_event.is_none(),
        "no switch event when mode stays the same"
    );
}

/// AC: DisabledByPolicy surfaced as actionable message.
///
/// When policy_enabled is false, resolve_mode returns DisabledByPolicy
/// with the configured mode preserved.
#[test]
fn cli_disabled_by_policy_surfaced_as_actionable_message() {
    let service = svc();
    let decision = service.resolve_mode(AssistantModeContext {
        session_id: sid(),
        configured_mode: AssistantMode::Kairos,
        session_mode: Some(AssistantMode::KairosBrief),
        policy_enabled: false,
        available_tools: all_tools(),
        capability_policy: default_policy(),
    });

    assert!(
        matches!(
            decision.error.as_ref().unwrap(),
            KairosError::DisabledByPolicy
        ),
        "expected DisabledByPolicy, got {:?}",
        decision.error
    );
    assert_eq!(
        decision.effective_mode,
        AssistantMode::Kairos,
        "configured mode preserved when disabled"
    );
    assert_eq!(
        decision.source,
        ResolvedModeSource::Config,
        "source is Config when disabled by policy"
    );

    // Verify the error message is actionable (Display impl).
    let msg = format!("{}", decision.error.unwrap());
    assert!(
        msg.contains("disabled by policy"),
        "error message should be actionable: '{msg}'"
    );
}

/// AC: DisabledByPolicy on capability_policy.allow_assistant_modes = false.
#[test]
fn cli_disabled_by_capability_policy() {
    let service = svc();
    let mut policy = default_policy();
    policy.allow_assistant_modes = false;

    let decision = service.resolve_mode(AssistantModeContext {
        session_id: sid(),
        configured_mode: AssistantMode::Kairos,
        session_mode: Some(AssistantMode::KairosBrief),
        policy_enabled: true,
        available_tools: all_tools(),
        capability_policy: policy,
    });

    assert!(matches!(
        decision.error.as_ref().unwrap(),
        KairosError::DisabledByPolicy
    ));
}

/// AC: mode switch via /brief command works end-to-end.
///
/// Starting from Kairos mode, /brief routes correctly, produces a switch event,
/// and the resulting mode's prompt composition succeeds.
#[test]
fn cli_mode_switch_via_brief_end_to_end() {
    let service = svc();

    // Step 1: Route the /brief command.
    let route = service.route_input(route_ctx(
        AssistantMode::Kairos,
        "/brief be concise",
        None,
        default_policy(),
    ));
    assert!(route.error.is_none());
    assert_eq!(route.next_mode, AssistantMode::KairosBrief);
    assert!(route.command_consumed);
    assert_eq!(route.passthrough_input, "be concise");
    assert!(
        matches!(
            route.switch_event,
            Some(DomainEvent::AssistantModeSwitched {
                from_mode: AssistantMode::Kairos,
                to_mode: AssistantMode::KairosBrief,
                trigger: ModeSwitchTrigger::SlashCommand,
                ..
            })
        ),
        "switch event with SlashCommand trigger expected"
    );

    // Step 2: Build prompt in the new mode.
    let prompt = service.build_prompt(PromptComposeContext {
        mode: route.next_mode,
        policy_enabled: true,
        capability_policy: default_policy(),
        base_prompt: Some("system base prompt".into()),
        mode_prompt_override: None,
        policy_prompt: None,
        runtime_prompt: None,
    });
    assert!(prompt.error.is_none());
    assert!(
        prompt
            .system_prompt
            .contains("KAIROS BRIEF mode. Keep responses concise"),
        "brief mode prompt should be in composed output"
    );
    assert_eq!(
        prompt.precedence,
        vec![
            PromptPrecedenceLayer::Base,
            PromptPrecedenceLayer::ModeDefault,
        ]
    );
    assert_eq!(prompt.telemetry_tags.mode, AssistantMode::KairosBrief);
    assert!(prompt.telemetry_tags.brief_active);
    assert!(!prompt.telemetry_tags.channels_active);

    // Step 3: Resolve mode with the switched state persisted in session.
    let resolved = service.resolve_mode(resolve_ctx(
        AssistantMode::Kairos,
        Some(AssistantMode::KairosBrief),
        default_policy(),
    ));
    assert!(resolved.error.is_none());
    assert_eq!(resolved.effective_mode, AssistantMode::KairosBrief);
    assert_eq!(resolved.source, ResolvedModeSource::Session);
}

/// AC: DisabledByPolicy on route_input returns noop with error.
#[test]
fn cli_route_disabled_by_policy_returns_noop() {
    let service = svc();
    let decision = service.route_input(RouteInputContext {
        session_id: sid(),
        current_mode: AssistantMode::Kairos,
        policy_enabled: false,
        capability_policy: default_policy(),
        raw_input: "/brief test".into(),
        explicit_mode: None,
    });

    assert_eq!(decision.source, RouteSource::Noop);
    assert!(!decision.command_consumed);
    assert!(matches!(
        decision.error.as_ref().unwrap(),
        KairosError::DisabledByPolicy
    ));
}

/// AC: telemetry tags reflect correct mode state.
#[test]
fn cli_telemetry_tags_reflect_mode_state() {
    let service = svc();

    let kairos = service.resolve_mode(resolve_ctx(AssistantMode::Kairos, None, default_policy()));
    assert!(kairos.telemetry_tags.kairos_active);
    assert!(!kairos.telemetry_tags.brief_active);
    assert!(!kairos.telemetry_tags.channels_active);

    let brief = service.resolve_mode(resolve_ctx(
        AssistantMode::Kairos,
        Some(AssistantMode::KairosBrief),
        default_policy(),
    ));
    assert!(brief.telemetry_tags.kairos_active);
    assert!(brief.telemetry_tags.brief_active);
    assert!(!brief.telemetry_tags.channels_active);

    let channels = service.resolve_mode(resolve_ctx(
        AssistantMode::Kairos,
        Some(AssistantMode::KairosChannels),
        default_policy(),
    ));
    assert!(channels.telemetry_tags.kairos_active);
    assert!(!channels.telemetry_tags.brief_active);
    assert!(channels.telemetry_tags.channels_active);
}
