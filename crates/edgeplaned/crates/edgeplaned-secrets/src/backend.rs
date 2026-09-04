use async_trait::async_trait;

use crate::{BackendCapabilities, BackendError, ResolveCtx, SecretRef, SecretValue};

/// The seam that makes secret backends pluggable. The default impl set is
/// env/literal/infisical; Vault and ProcessBackend arrive later behind this
/// same trait. Mandatory: `resolve` + `health`. Optional methods are gated by
/// `capabilities()` and error `Unsupported` by default (Guard 1).
#[async_trait]
pub trait SecretsBackend: Send + Sync {
    fn scheme(&self) -> &str;

    async fn resolve(&self, r: &SecretRef, ctx: &ResolveCtx) -> Result<SecretValue, BackendError>;

    async fn health(&self) -> Result<(), BackendError>;

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::default()
    }

    async fn mint_scoped_identity(&self, _agent_id: &str) -> Result<String, BackendError> {
        Err(BackendError::Unsupported("scoped_identity"))
    }

    async fn mint_lease(
        &self,
        _r: &SecretRef,
        _ttl_secs: u64,
    ) -> Result<SecretValue, BackendError> {
        Err(BackendError::Unsupported("leasing"))
    }

    async fn revoke(&self, _handle: &str) -> Result<(), BackendError> {
        Err(BackendError::Unsupported("revoke"))
    }
}
