pub mod cron;
pub mod embedding;
pub mod event_bus;
pub mod memory;
pub mod provider;
pub mod repositories;
pub mod tool;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PortError {
    #[error("not found")]
    NotFound,
    #[error("storage error: {0}")]
    Storage(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("channel error: {0}")]
    Channel(String),
    #[error("tool error: {0}")]
    Tool(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
}
