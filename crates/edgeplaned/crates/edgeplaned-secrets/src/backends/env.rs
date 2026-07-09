use async_trait::async_trait;

use crate::backend::SecretsBackend;
use crate::{BackendError, ResolveCtx, SecretRef, SecretValue};

pub struct EnvBackend;

impl EnvBackend {
    pub fn new() -> Self {
        EnvBackend
    }
}

impl Default for EnvBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SecretsBackend for EnvBackend {
    fn scheme(&self) -> &str {
        "env"
    }

    async fn resolve(&self, r: &SecretRef, _ctx: &ResolveCtx) -> Result<SecretValue, BackendError> {
        std::env::var(&r.path)
            .map(SecretValue::new)
            .map_err(|_| BackendError::NotFound(format!("env var {}", r.path)))
    }

    async fn health(&self) -> Result<(), BackendError> {
        Ok(())
    }
}
