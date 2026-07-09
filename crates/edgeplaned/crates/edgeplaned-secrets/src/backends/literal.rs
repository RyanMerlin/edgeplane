use async_trait::async_trait;
use std::collections::HashMap;

use crate::backend::SecretsBackend;
use crate::{BackendError, ResolveCtx, SecretRef, SecretValue};

pub struct LiteralBackend {
    values: HashMap<String, String>,
}

impl LiteralBackend {
    pub fn new(values: HashMap<String, String>) -> Self {
        LiteralBackend { values }
    }
}

#[async_trait]
impl SecretsBackend for LiteralBackend {
    fn scheme(&self) -> &str {
        "literal"
    }

    async fn resolve(&self, r: &SecretRef, _ctx: &ResolveCtx) -> Result<SecretValue, BackendError> {
        self.values
            .get(&r.path)
            .cloned()
            .map(SecretValue::new)
            .ok_or_else(|| BackendError::NotFound(format!("literal {}", r.path)))
    }

    async fn health(&self) -> Result<(), BackendError> {
        Ok(())
    }
}
