use thiserror::Error;

#[derive(Debug, Error)]
pub enum PacksError {
    #[error("YAML parse error in {file}: {source}")]
    YamlParse {
        file: String,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("capability '{0}' not found")]
    CapabilityNotFound(String),

    #[error("pack '{0}' not found")]
    PackNotFound(String),

    #[error("invalid capability name '{0}': expected 'pack.capability' format")]
    InvalidCapabilityName(String),
}

pub type Result<T> = std::result::Result<T, PacksError>;
