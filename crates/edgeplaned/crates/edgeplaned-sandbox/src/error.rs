use thiserror::Error;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("sandbox error: {0}")]
    Sandbox(String),

    #[error("isolation error: {0}")]
    Isolation(String),

    #[error("integrity failure: {0}")]
    IntegrityFailure(String),
}

pub type Result<T> = std::result::Result<T, SandboxError>;
