use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("port error: {0}")]
    Port(#[from] ccode_ports::PortError),
    #[error("domain error: {0}")]
    Domain(#[from] ccode_domain::error::DomainError),
}
