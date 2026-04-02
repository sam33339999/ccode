use serde::{Deserialize, Serialize};

/// Lifecycle state of a single agent turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AgentState {
    /// Waiting for user input.
    Idle,
    /// Waiting for the LLM to respond.
    Running,
    /// Turn completed successfully.
    Done,
    /// Turn ended with an error.
    Failed { reason: String },
}
