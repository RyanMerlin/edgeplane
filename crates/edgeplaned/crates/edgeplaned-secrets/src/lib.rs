pub mod config;
pub mod error;
pub mod types;
pub mod client;
pub mod redact;
pub mod resolver;
pub mod session;
pub mod token_cache;

#[cfg(target_os = "linux")]
pub mod keyring;

pub use config::{InfisicalConfig, InfisicalProfileMap, migrate_legacy};
pub use client::InfisicalClient;
pub use error::{SecretsError, Result};
pub use types::{CredentialSource, CredentialKind, ResolvedCredentials};
pub use redact::SecretRedactor;
pub use resolver::{resolve_credentials, resolve_credentials_with_profiles};
pub use token_cache::TokenCache;
pub use session::SessionStore;

#[cfg(target_os = "linux")]
pub use keyring::{
    store_service_token, load_service_token, delete_service_token, migrate_legacy_entry,
    KeyringResult,
};
