//! MCP runtime crate placeholder.
pub mod client;
pub mod contracts;
pub mod transport;

#[cfg(test)]
mod chicago_mcp_acceptance_tests;
#[cfg(test)]
mod client_tests;
#[cfg(test)]
mod cross_feature_integration_tests;
#[cfg(test)]
mod transport_tests;
