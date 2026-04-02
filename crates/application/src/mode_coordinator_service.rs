use crate::spec_contracts::{
    CoordinatorMode, CoordinatorModeError, EffectiveMode, EffectiveModeSource,
    ModeCoordinatorService, ModeReconcileAction, ModeReconcileResult, ModeResolutionInput,
    ModeSwitchEvent, ModeSwitchReason, SessionMode,
};

#[derive(Debug, Clone, Copy)]
pub struct DefaultModeCoordinatorService {
    pub policy_enabled: bool,
    pub env_mode: Option<CoordinatorMode>,
    pub current_mode: CoordinatorMode,
}

impl Default for DefaultModeCoordinatorService {
    fn default() -> Self {
        Self {
            policy_enabled: true,
            env_mode: None,
            current_mode: CoordinatorMode::Standard,
        }
    }
}

impl DefaultModeCoordinatorService {
    pub fn from_env(policy_enabled: bool, current_mode: CoordinatorMode) -> Self {
        Self {
            policy_enabled,
            env_mode: std::env::var(crate::spec_contracts::CLAUDE_CODE_COORDINATOR_MODE)
                .ok()
                .as_deref()
                .and_then(CoordinatorMode::parse),
            current_mode,
        }
    }

    fn transition_allowed(from: CoordinatorMode, to: CoordinatorMode) -> bool {
        // Coordinator mode is currently sticky once entered to keep orchestration behavior stable.
        !matches!(
            (from, to),
            (CoordinatorMode::Coordinator, CoordinatorMode::Standard)
        )
    }

    fn mode_event(
        from: CoordinatorMode,
        to: CoordinatorMode,
        reason: ModeSwitchReason,
    ) -> Option<ModeSwitchEvent> {
        (from != to).then_some(ModeSwitchEvent { from, to, reason })
    }
}

impl ModeCoordinatorService for DefaultModeCoordinatorService {
    fn resolve_effective_mode(&self, input: ModeResolutionInput) -> EffectiveMode {
        if !input.policy_enabled {
            return EffectiveMode {
                mode: CoordinatorMode::Standard,
                source: EffectiveModeSource::Default,
                switch_event: None,
                error: Some(CoordinatorModeError::DisabledByPolicy),
            };
        }

        let (candidate_mode, source, reason) = if let Some(session_mode) = input.session_mode {
            (
                session_mode.mode,
                EffectiveModeSource::Session,
                ModeSwitchReason::SessionPrecedence,
            )
        } else if let Some(env_mode) = input.env_mode {
            (
                env_mode,
                EffectiveModeSource::Env,
                ModeSwitchReason::EnvConfigured,
            )
        } else {
            (
                CoordinatorMode::Standard,
                EffectiveModeSource::Default,
                ModeSwitchReason::DefaultFallback,
            )
        };

        if let Some(previous_mode) = input.previous_mode
            && !Self::transition_allowed(previous_mode, candidate_mode)
        {
            return EffectiveMode {
                mode: previous_mode,
                source,
                switch_event: None,
                error: Some(CoordinatorModeError::InvalidModeTransition),
            };
        }

        EffectiveMode {
            mode: candidate_mode,
            source,
            switch_event: input
                .previous_mode
                .and_then(|previous| Self::mode_event(previous, candidate_mode, reason)),
            error: None,
        }
    }

    fn reconcile_on_resume(&self, session_mode: Option<SessionMode>) -> ModeReconcileResult {
        if !self.policy_enabled {
            return ModeReconcileResult {
                mode: CoordinatorMode::Standard,
                action: ModeReconcileAction::Noop,
                switch_event: None,
                error: Some(CoordinatorModeError::DisabledByPolicy),
            };
        }

        if let Some(session_mode) = session_mode {
            if !Self::transition_allowed(self.current_mode, session_mode.mode) {
                return ModeReconcileResult {
                    mode: self.current_mode,
                    action: ModeReconcileAction::Noop,
                    switch_event: None,
                    error: Some(CoordinatorModeError::InvalidModeTransition),
                };
            }

            let conflict = self.env_mode.is_some_and(|env| env != session_mode.mode);
            return ModeReconcileResult {
                mode: session_mode.mode,
                action: ModeReconcileAction::SessionWins,
                switch_event: Self::mode_event(
                    self.current_mode,
                    session_mode.mode,
                    ModeSwitchReason::ResumeReconcile,
                ),
                error: conflict.then_some(CoordinatorModeError::SessionModeMismatch),
            };
        }

        let target_mode = self.env_mode.unwrap_or(CoordinatorMode::Standard);
        if !Self::transition_allowed(self.current_mode, target_mode) {
            return ModeReconcileResult {
                mode: self.current_mode,
                action: ModeReconcileAction::Noop,
                switch_event: None,
                error: Some(CoordinatorModeError::InvalidModeTransition),
            };
        }

        ModeReconcileResult {
            mode: target_mode,
            action: ModeReconcileAction::EnvAdopted,
            switch_event: Self::mode_event(
                self.current_mode,
                target_mode,
                ModeSwitchReason::ResumeReconcile,
            ),
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec_contracts::ModeCoordinatorService;

    #[test]
    fn resolve_effective_mode_prefers_session_over_env_mode() {
        let service = DefaultModeCoordinatorService::default();
        let result = service.resolve_effective_mode(ModeResolutionInput {
            policy_enabled: true,
            env_mode: Some(CoordinatorMode::Standard),
            session_mode: Some(SessionMode {
                mode: CoordinatorMode::Coordinator,
            }),
            previous_mode: Some(CoordinatorMode::Standard),
        });

        assert_eq!(result.mode, CoordinatorMode::Coordinator);
        assert_eq!(result.source, EffectiveModeSource::Session);
        assert!(result.error.is_none());
        assert!(matches!(
            result.switch_event,
            Some(ModeSwitchEvent {
                from: CoordinatorMode::Standard,
                to: CoordinatorMode::Coordinator,
                reason: ModeSwitchReason::SessionPrecedence,
            })
        ));
    }

    #[test]
    fn reconcile_on_resume_uses_session_mode_and_marks_conflict_deterministically() {
        let service = DefaultModeCoordinatorService {
            policy_enabled: true,
            env_mode: Some(CoordinatorMode::Standard),
            current_mode: CoordinatorMode::Standard,
        };

        let result = service.reconcile_on_resume(Some(SessionMode {
            mode: CoordinatorMode::Coordinator,
        }));

        assert_eq!(result.mode, CoordinatorMode::Coordinator);
        assert_eq!(result.action, ModeReconcileAction::SessionWins);
        assert!(matches!(
            result.error,
            Some(CoordinatorModeError::SessionModeMismatch)
        ));
        assert!(matches!(
            result.switch_event,
            Some(ModeSwitchEvent {
                from: CoordinatorMode::Standard,
                to: CoordinatorMode::Coordinator,
                reason: ModeSwitchReason::ResumeReconcile,
            })
        ));
    }

    #[test]
    fn resolve_effective_mode_reports_disabled_by_policy() {
        let service = DefaultModeCoordinatorService::default();
        let result = service.resolve_effective_mode(ModeResolutionInput {
            policy_enabled: false,
            env_mode: Some(CoordinatorMode::Coordinator),
            session_mode: Some(SessionMode {
                mode: CoordinatorMode::Coordinator,
            }),
            previous_mode: Some(CoordinatorMode::Standard),
        });

        assert!(matches!(
            result.error,
            Some(CoordinatorModeError::DisabledByPolicy)
        ));
    }

    #[test]
    fn resolve_effective_mode_reports_invalid_transition() {
        let service = DefaultModeCoordinatorService::default();
        let result = service.resolve_effective_mode(ModeResolutionInput {
            policy_enabled: true,
            env_mode: Some(CoordinatorMode::Standard),
            session_mode: None,
            previous_mode: Some(CoordinatorMode::Coordinator),
        });

        assert!(matches!(
            result.error,
            Some(CoordinatorModeError::InvalidModeTransition)
        ));
    }

    #[test]
    fn reconcile_on_resume_can_surface_each_error_variant() {
        let disabled = DefaultModeCoordinatorService {
            policy_enabled: false,
            env_mode: Some(CoordinatorMode::Coordinator),
            current_mode: CoordinatorMode::Standard,
        }
        .reconcile_on_resume(Some(SessionMode {
            mode: CoordinatorMode::Coordinator,
        }));
        assert!(matches!(
            disabled.error,
            Some(CoordinatorModeError::DisabledByPolicy)
        ));

        let invalid = DefaultModeCoordinatorService {
            policy_enabled: true,
            env_mode: Some(CoordinatorMode::Standard),
            current_mode: CoordinatorMode::Coordinator,
        }
        .reconcile_on_resume(None);
        assert!(matches!(
            invalid.error,
            Some(CoordinatorModeError::InvalidModeTransition)
        ));

        let mismatch = DefaultModeCoordinatorService {
            policy_enabled: true,
            env_mode: Some(CoordinatorMode::Standard),
            current_mode: CoordinatorMode::Standard,
        }
        .reconcile_on_resume(Some(SessionMode {
            mode: CoordinatorMode::Coordinator,
        }));
        assert!(matches!(
            mismatch.error,
            Some(CoordinatorModeError::SessionModeMismatch)
        ));
    }
}
