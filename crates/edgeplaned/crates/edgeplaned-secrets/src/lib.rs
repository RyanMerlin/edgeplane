pub mod backend;
pub mod backend_types;
pub mod backends;
pub mod client;
pub mod config;
pub mod error;
pub mod redact;
pub mod registry;
pub mod resolver;
pub mod secret_ref;
pub mod session;
pub mod token_cache;
pub mod types;

#[cfg(target_os = "linux")]
pub mod keyring;

pub use backend::SecretsBackend;
pub use backend_types::{BackendCapabilities, BackendError, ResolveCtx, SecretValue};
pub use backends::InfisicalBackend;
pub use backends::{EnvBackend, LiteralBackend};
pub use client::InfisicalClient;
pub use config::{migrate_legacy, InfisicalConfig, InfisicalProfileMap};
pub use error::{Result, SecretsError};
pub use redact::SecretRedactor;
pub use registry::BackendRegistry;
pub use resolver::{
    resolve_credentials, resolve_credentials_with_profiles, resolve_credentials_with_registry,
};
pub use secret_ref::SecretRef;
pub use session::SessionStore;
pub use token_cache::TokenCache;
pub use types::{CredentialKind, CredentialSource, ResolvedCredentials};

#[cfg(target_os = "linux")]
pub use keyring::{
    delete_service_token, load_service_token, migrate_legacy_entry, store_service_token,
    KeyringResult,
};
