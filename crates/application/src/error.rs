use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("port error: {0}")]
    Port(#[from] ccode_ports::PortError),
    #[error("llm error: {0}")]
    Llm(#[from] ccode_ports::provider::LlmError),
    #[error("domain error: {0}")]
    Domain(#[from] ccode_domain::error::DomainError),
    #[error("vision is not supported by provider: {0}")]
    VisionNotSupported(String),
}
