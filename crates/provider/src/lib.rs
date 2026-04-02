pub mod contracts;
pub mod factory;
pub mod router;

/// Wire-type conversion between Anthropic and OpenAI formats.
/// Internal to provider crate — upper layers use `ccode_ports` canonical types.
pub mod compat;

pub(crate) mod openai_compat;

#[cfg(feature = "provider-anthropic")]
pub(crate) mod anthropic_compat;

#[cfg(feature = "provider-openrouter")]
pub mod openrouter;

#[cfg(feature = "provider-zhipu")]
pub mod zhipu;

#[cfg(feature = "provider-llamacpp")]
pub mod llamacpp;

#[cfg(feature = "provider-anthropic")]
pub mod anthropic;
