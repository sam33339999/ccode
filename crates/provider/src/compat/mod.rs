//! Wire-type definitions and bidirectional conversion between
//! Anthropic Messages API and OpenAI Chat Completions API formats.
//!
//! These types are internal to the provider crate — upper layers
//! interact only with `ccode_ports::provider::*` canonical types.

pub mod convert;
pub mod request;
pub mod response;

pub use convert::*;
pub use request::*;
pub use response::*;
