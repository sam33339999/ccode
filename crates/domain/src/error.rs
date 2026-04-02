use thiserror::Error;
use crate::session::SessionId;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("session not found: {0}")]
    SessionNotFound(SessionId),
    #[error("invalid state: {0}")]
    InvalidState(String),
}
