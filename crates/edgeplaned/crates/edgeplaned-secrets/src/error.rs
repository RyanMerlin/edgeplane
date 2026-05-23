use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecretsError {
    #[error("HTTP error: {0}")]
    Http(String),

    #[error("JSON error: {0}")]
    Json(String),

    #[error("Keyring error: {0}")]
    Keyring(String),

    #[error("No service token configured — store one with store_service_token()")]
    TokenMissing,

    #[error("environment variable `{0}` is not set")]
    EnvVarMissing(String),

    #[error("Secret not found: {0}")]
    SecretNotFound(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Configuration error: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, SecretsError>;
