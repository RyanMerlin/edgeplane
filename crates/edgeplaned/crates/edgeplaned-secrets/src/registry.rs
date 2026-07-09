use std::collections::HashMap;
use std::sync::Arc;

use crate::backend::SecretsBackend;
use crate::{BackendError, ResolveCtx, SecretRef, SecretValue};

#[derive(Default, Clone)]
pub struct BackendRegistry {
    backends: HashMap<String, Arc<dyn SecretsBackend>>,
}

impl BackendRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn register(&mut self, backend: Arc<dyn SecretsBackend>) {
        self.backends.insert(backend.scheme().to_string(), backend);
    }

    pub fn get(&self, scheme: &str) -> Option<&Arc<dyn SecretsBackend>> {
        self.backends.get(scheme)
    }

    pub async fn resolve(&self, r: &SecretRef, ctx: &ResolveCtx) -> Result<SecretValue, BackendError> {
        match self.backends.get(&r.scheme) {
            Some(b) => b.resolve(r, ctx).await,
            None => Err(BackendError::Unavailable(format!("no backend for scheme '{}'", r.scheme))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::{EnvBackend, SecretRef, ResolveCtx};

    #[tokio::test]
    async fn routes_by_scheme() {
        std::env::set_var("EP_SECRETS_T5", "reg-val");
        let mut reg = BackendRegistry::new();
        reg.register(Arc::new(EnvBackend::new()));
        let r = SecretRef::parse("secret://env/EP_SECRETS_T5").unwrap();
        assert_eq!(reg.resolve(&r, &ResolveCtx::default()).await.unwrap().expose(), "reg-val");
        std::env::remove_var("EP_SECRETS_T5");
    }

    #[tokio::test]
    async fn unknown_scheme_errors() {
        let reg = BackendRegistry::new();
        let r = SecretRef::parse("secret://nope/x").unwrap();
        assert!(matches!(reg.resolve(&r, &ResolveCtx::default()).await,
            Err(crate::BackendError::Unavailable(_))));
    }
}
