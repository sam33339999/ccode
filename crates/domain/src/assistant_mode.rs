use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssistantMode {
    Kairos,
    KairosBrief,
    KairosChannels,
}

impl AssistantMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kairos => "KAIROS",
            Self::KairosBrief => "KAIROS_BRIEF",
            Self::KairosChannels => "KAIROS_CHANNELS",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModeSwitchTrigger {
    ConfigDefault,
    SessionState,
    SlashCommand,
    ExplicitOverride,
}

impl ModeSwitchTrigger {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConfigDefault => "config_default",
            Self::SessionState => "session_state",
            Self::SlashCommand => "slash_command",
            Self::ExplicitOverride => "explicit_override",
        }
    }
}
