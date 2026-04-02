use crate::spec_contracts::{
    AssistantModeContext, AssistantModeDecision, AssistantModeService, CapabilityPolicy,
    KairosError, KairosTelemetryTags, PromptComposeContext, PromptComposeResult,
    PromptPrecedenceLayer, ResolvedModeSource, RouteDecision, RouteInputContext, RouteSource,
};
use ccode_domain::{
    assistant_mode::{AssistantMode, ModeSwitchTrigger},
    event::DomainEvent,
    session::SessionId,
};
use std::collections::BTreeSet;

#[derive(Debug, Default)]
pub struct DefaultAssistantModeService;

impl AssistantModeService for DefaultAssistantModeService {
    fn resolve_mode(&self, ctx: AssistantModeContext) -> AssistantModeDecision {
        if !ctx.policy_enabled || !ctx.capability_policy.allow_assistant_modes {
            return AssistantModeDecision {
                effective_mode: ctx.configured_mode,
                source: ResolvedModeSource::Config,
                switch_event: None,
                visible_tools: Vec::new(),
                telemetry_tags: Self::telemetry(
                    ctx.configured_mode,
                    ModeSwitchTrigger::ConfigDefault,
                    0,
                ),
                error: Some(KairosError::DisabledByPolicy),
            };
        }

        let (effective_mode, source, trigger) = match ctx.session_mode {
            Some(mode) => (
                mode,
                ResolvedModeSource::Session,
                ModeSwitchTrigger::SessionState,
            ),
            None => (
                ctx.configured_mode,
                ResolvedModeSource::Config,
                ModeSwitchTrigger::ConfigDefault,
            ),
        };

        if !Self::mode_allowed(effective_mode, &ctx.capability_policy) {
            return AssistantModeDecision {
                effective_mode: ctx.configured_mode,
                source: ResolvedModeSource::Config,
                switch_event: None,
                visible_tools: Vec::new(),
                telemetry_tags: Self::telemetry(
                    ctx.configured_mode,
                    ModeSwitchTrigger::ConfigDefault,
                    0,
                ),
                error: Some(KairosError::InvalidModeState),
            };
        }

        AssistantModeDecision {
            effective_mode,
            source,
            switch_event: Self::mode_switch_event(
                &ctx.session_id,
                ctx.configured_mode,
                effective_mode,
                trigger,
            ),
            visible_tools: Self::visible_tools(
                effective_mode,
                &ctx.available_tools,
                &ctx.capability_policy,
            ),
            telemetry_tags: Self::telemetry(effective_mode, trigger, 0),
            error: None,
        }
    }

    fn build_prompt(&self, ctx: PromptComposeContext) -> PromptComposeResult {
        if !ctx.policy_enabled || !ctx.capability_policy.allow_assistant_modes {
            return PromptComposeResult {
                system_prompt: String::new(),
                precedence: Vec::new(),
                telemetry_tags: Self::telemetry(ctx.mode, ModeSwitchTrigger::ConfigDefault, 0),
                error: Some(KairosError::DisabledByPolicy),
            };
        }
        if !Self::mode_allowed(ctx.mode, &ctx.capability_policy) {
            return PromptComposeResult {
                system_prompt: String::new(),
                precedence: Vec::new(),
                telemetry_tags: Self::telemetry(ctx.mode, ModeSwitchTrigger::ConfigDefault, 0),
                error: Some(KairosError::InvalidModeState),
            };
        }

        let mut precedence = Vec::new();
        let mut layers = Vec::new();

        if let Some(base) = ctx
            .base_prompt
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            precedence.push(PromptPrecedenceLayer::Base);
            layers.push(base.to_owned());
        }

        precedence.push(PromptPrecedenceLayer::ModeDefault);
        layers.push(Self::default_mode_prompt(ctx.mode).to_owned());

        if let Some(mode_override) = ctx
            .mode_prompt_override
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            precedence.push(PromptPrecedenceLayer::ModeOverride);
            layers.push(mode_override.to_owned());
        }

        if let Some(policy_prompt) = ctx
            .policy_prompt
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            precedence.push(PromptPrecedenceLayer::Policy);
            layers.push(policy_prompt.to_owned());
        }

        if let Some(runtime_prompt) = ctx
            .runtime_prompt
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            precedence.push(PromptPrecedenceLayer::Runtime);
            layers.push(runtime_prompt.to_owned());
        }

        let system_prompt = layers.join("\n\n");
        if system_prompt.is_empty() {
            return PromptComposeResult {
                system_prompt,
                precedence,
                telemetry_tags: Self::telemetry(ctx.mode, ModeSwitchTrigger::ConfigDefault, 0),
                error: Some(KairosError::PromptComposeFailed(
                    "no prompt layers available".to_owned(),
                )),
            };
        }

        let layer_count = precedence.len();
        PromptComposeResult {
            system_prompt,
            precedence,
            telemetry_tags: Self::telemetry(
                ctx.mode,
                ModeSwitchTrigger::ConfigDefault,
                layer_count,
            ),
            error: None,
        }
    }

    fn route_input(&self, ctx: RouteInputContext) -> RouteDecision {
        if !ctx.policy_enabled || !ctx.capability_policy.allow_assistant_modes {
            return RouteDecision {
                source: RouteSource::Noop,
                next_mode: ctx.current_mode,
                passthrough_input: ctx.raw_input,
                command_consumed: false,
                switch_event: None,
                telemetry_tags: Self::telemetry(
                    ctx.current_mode,
                    ModeSwitchTrigger::ConfigDefault,
                    0,
                ),
                error: Some(KairosError::DisabledByPolicy),
            };
        }

        let slash = Self::parse_slash_mode(&ctx.raw_input);
        let slash_mode = slash.as_ref().map(|(mode, _)| *mode);

        if let (Some(explicit), Some(from_slash)) = (ctx.explicit_mode, slash_mode)
            && explicit != from_slash
        {
            return RouteDecision {
                source: RouteSource::Conflict,
                next_mode: ctx.current_mode,
                passthrough_input: ctx.raw_input,
                command_consumed: false,
                switch_event: None,
                telemetry_tags: Self::telemetry(
                    ctx.current_mode,
                    ModeSwitchTrigger::ConfigDefault,
                    0,
                ),
                error: Some(KairosError::RouteConflict),
            };
        }

        let next_mode = ctx.explicit_mode.or(slash_mode).unwrap_or(ctx.current_mode);

        if !Self::mode_allowed(next_mode, &ctx.capability_policy) {
            return RouteDecision {
                source: RouteSource::Noop,
                next_mode: ctx.current_mode,
                passthrough_input: ctx.raw_input,
                command_consumed: false,
                switch_event: None,
                telemetry_tags: Self::telemetry(
                    ctx.current_mode,
                    ModeSwitchTrigger::ConfigDefault,
                    0,
                ),
                error: Some(KairosError::InvalidModeState),
            };
        }

        let source = if ctx.explicit_mode.is_some() {
            RouteSource::ExplicitOverride
        } else if slash_mode.is_some() {
            RouteSource::SlashCommand
        } else {
            RouteSource::Noop
        };
        let trigger = match source {
            RouteSource::ExplicitOverride => ModeSwitchTrigger::ExplicitOverride,
            RouteSource::SlashCommand => ModeSwitchTrigger::SlashCommand,
            RouteSource::Noop | RouteSource::Conflict => ModeSwitchTrigger::ConfigDefault,
        };

        let (command_consumed, passthrough_input) = if let Some((_, rest)) = slash {
            (true, rest)
        } else {
            (false, ctx.raw_input)
        };

        RouteDecision {
            source,
            next_mode,
            passthrough_input,
            command_consumed,
            switch_event: Self::mode_switch_event(
                &ctx.session_id,
                ctx.current_mode,
                next_mode,
                trigger,
            ),
            telemetry_tags: Self::telemetry(next_mode, trigger, 0),
            error: None,
        }
    }
}

impl DefaultAssistantModeService {
    fn mode_allowed(mode: AssistantMode, capability_policy: &CapabilityPolicy) -> bool {
        match mode {
            AssistantMode::Kairos => capability_policy.allow_assistant_modes,
            AssistantMode::KairosBrief => {
                capability_policy.allow_assistant_modes && capability_policy.allow_brief_mode
            }
            AssistantMode::KairosChannels => {
                capability_policy.allow_assistant_modes && capability_policy.allow_channels_mode
            }
        }
    }

    fn default_mode_prompt(mode: AssistantMode) -> &'static str {
        match mode {
            AssistantMode::Kairos => "You are KAIROS mode. Balance depth and efficiency.",
            AssistantMode::KairosBrief => {
                "You are KAIROS BRIEF mode. Keep responses concise and focused."
            }
            AssistantMode::KairosChannels => {
                "You are KAIROS CHANNELS mode. Coordinate multi-channel context safely."
            }
        }
    }

    fn mode_switch_event(
        session_id: &SessionId,
        from_mode: AssistantMode,
        to_mode: AssistantMode,
        trigger: ModeSwitchTrigger,
    ) -> Option<DomainEvent> {
        (from_mode != to_mode).then(|| DomainEvent::AssistantModeSwitched {
            session_id: session_id.clone(),
            from_mode,
            to_mode,
            trigger,
        })
    }

    fn telemetry(
        mode: AssistantMode,
        mode_source: ModeSwitchTrigger,
        prompt_layer_count: usize,
    ) -> KairosTelemetryTags {
        KairosTelemetryTags {
            mode,
            mode_source,
            kairos_active: true,
            brief_active: mode == AssistantMode::KairosBrief,
            channels_active: mode == AssistantMode::KairosChannels,
            prompt_layer_count,
        }
    }

    fn parse_slash_mode(raw_input: &str) -> Option<(AssistantMode, String)> {
        let trimmed = raw_input.trim_start();
        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let command = parts.next()?;
        let remainder = parts
            .next()
            .map(str::trim_start)
            .unwrap_or_default()
            .to_owned();
        let mode = match command {
            "/brief" => AssistantMode::KairosBrief,
            "/kairos" => AssistantMode::Kairos,
            "/channels" => AssistantMode::KairosChannels,
            _ => return None,
        };
        Some((mode, remainder))
    }

    pub fn visible_tools(
        mode: AssistantMode,
        available_tools: &[String],
        capability_policy: &CapabilityPolicy,
    ) -> Vec<String> {
        let blocked: BTreeSet<&str> = capability_policy
            .blocked_tools
            .iter()
            .map(String::as_str)
            .collect();
        let brief_blocked: BTreeSet<&str> = capability_policy
            .brief_blocked_tools
            .iter()
            .map(String::as_str)
            .collect();
        available_tools
            .iter()
            .filter(|tool| !blocked.contains(tool.as_str()))
            .filter(|tool| {
                mode != AssistantMode::KairosBrief || !brief_blocked.contains(tool.as_str())
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec_contracts::{
        CapabilityPolicy, PromptPrecedenceLayer, ResolvedModeSource, RouteSource,
    };
    use ccode_domain::{assistant_mode::AssistantMode, event::DomainEvent, session::SessionId};

    #[test]
    fn resolve_mode_is_deterministic_and_emits_mode_switch_event() {
        let service = DefaultAssistantModeService;
        let decision = service.resolve_mode(AssistantModeContext {
            session_id: SessionId("sess-1".into()),
            configured_mode: AssistantMode::Kairos,
            session_mode: Some(AssistantMode::KairosBrief),
            policy_enabled: true,
            available_tools: vec!["read_file".into(), "shell".into()],
            capability_policy: CapabilityPolicy::default(),
        });

        assert!(decision.error.is_none());
        assert_eq!(decision.effective_mode, AssistantMode::KairosBrief);
        assert_eq!(decision.source, ResolvedModeSource::Session);
        assert!(matches!(
            decision.switch_event,
            Some(DomainEvent::AssistantModeSwitched {
                from_mode: AssistantMode::Kairos,
                to_mode: AssistantMode::KairosBrief,
                ..
            })
        ));
    }

    #[test]
    fn build_prompt_applies_deterministic_precedence_order() {
        let service = DefaultAssistantModeService;
        let result = service.build_prompt(PromptComposeContext {
            mode: AssistantMode::KairosBrief,
            policy_enabled: true,
            capability_policy: CapabilityPolicy::default(),
            base_prompt: Some("base".into()),
            mode_prompt_override: Some("mode_override".into()),
            policy_prompt: Some("policy".into()),
            runtime_prompt: Some("runtime".into()),
        });

        assert!(result.error.is_none());
        assert_eq!(
            result.precedence,
            vec![
                PromptPrecedenceLayer::Base,
                PromptPrecedenceLayer::ModeDefault,
                PromptPrecedenceLayer::ModeOverride,
                PromptPrecedenceLayer::Policy,
                PromptPrecedenceLayer::Runtime,
            ]
        );
        assert_eq!(
            result.system_prompt,
            "base\n\nYou are KAIROS BRIEF mode. Keep responses concise and focused.\n\nmode_override\n\npolicy\n\nruntime"
        );
    }

    #[test]
    fn route_input_reports_conflict_when_override_and_slash_disagree() {
        let service = DefaultAssistantModeService;
        let decision = service.route_input(RouteInputContext {
            session_id: SessionId("sess-1".into()),
            current_mode: AssistantMode::Kairos,
            policy_enabled: true,
            capability_policy: CapabilityPolicy::default(),
            raw_input: "/brief summarize this".into(),
            explicit_mode: Some(AssistantMode::KairosChannels),
        });

        assert_eq!(decision.source, RouteSource::Conflict);
        assert!(decision.error.is_some());
        assert!(decision.switch_event.is_none());
    }
}
