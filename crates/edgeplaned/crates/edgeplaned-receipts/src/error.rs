use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReceiptsError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("receipt not found: {0}")]
    NotFound(String),
}
