use async_trait::async_trait;

use crate::backend::SecretsBackend;
use crate::client::InfisicalClient;
use crate::config::InfisicalConfig;
use crate::error::SecretsError;
use crate::{BackendCapabilities, BackendError, ResolveCtx, SecretRef, SecretValue};

pub struct InfisicalBackend {
    client: InfisicalClient,
    default_env: String,
    default_project: Option<String>,
}

/// Resolved fetch arguments — the mapping from a SecretRef to the HTTP client call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FetchArgs {
    pub secret_name: String,
    pub secret_path: String,
    pub environment: String,
    pub project_id: Option<String>,
}

impl InfisicalBackend {
    pub fn new(cfg: InfisicalConfig) -> std::result::Result<Self, SecretsError> {
        let default_env = cfg.default_environment.clone();
        let default_project = cfg.default_project_id.clone();
        let client = InfisicalClient::new(&cfg)?;
        Ok(InfisicalBackend {
            client,
            default_env,
            default_project,
        })
    }

    /// Split `<path...>/<SECRET_NAME>` and apply query overrides. Pure — unit-tested.
    pub(crate) fn map_ref(
        r: &SecretRef,
        default_env: &str,
        default_project: Option<&str>,
    ) -> FetchArgs {
        let (secret_path, secret_name) = match r.path.rsplit_once('/') {
            Some((dir, name)) => (format!("/{dir}"), name.to_string()),
            None => ("/".to_string(), r.path.clone()),
        };
        let environment = r
            .query
            .get("env")
            .cloned()
            .unwrap_or_else(|| default_env.to_string());
        let project_id = r
            .query
            .get("project")
            .cloned()
            .or_else(|| default_project.map(str::to_string));

        FetchArgs {
            secret_name,
            secret_path,
            environment,
            project_id,
        }
    }
}

#[async_trait]
impl SecretsBackend for InfisicalBackend {
    fn scheme(&self) -> &str {
        "infisical"
    }

    async fn resolve(&self, r: &SecretRef, _ctx: &ResolveCtx) -> Result<SecretValue, BackendError> {
        let mapped = Self::map_ref(r, &self.default_env, self.default_project.as_deref());
        let project_id = mapped.project_id.as_deref().unwrap_or("");
        self.client
            .fetch_secret(
                &mapped.secret_name,
                project_id,
                &mapped.environment,
                &mapped.secret_path,
            )
            .await
            .map(SecretValue::new)
            .map_err(|e| BackendError::Other(e.to_string()))
    }

    async fn health(&self) -> Result<(), BackendError> {
        let project_id = self.default_project.as_deref().unwrap_or("");
        self.client
            .list_folders(project_id, &self.default_env, "/")
            .await
            .map(|_| ())
            .map_err(|e| BackendError::Unavailable(e.to_string()))
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            scoped_identity: true,
            writeback: true,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SecretRef;

    #[test]
    fn ref_maps_to_fetch_args_with_defaults() {
        let r = SecretRef::parse("secret://infisical/providers/openai/OPENAI_API_KEY").unwrap();
        let m = InfisicalBackend::map_ref(&r, "prod", Some("projX"));
        assert_eq!(m.secret_name, "OPENAI_API_KEY");
        assert_eq!(m.secret_path, "/providers/openai");
        assert_eq!(m.environment, "prod");
        assert_eq!(m.project_id.as_deref(), Some("projX"));
    }

    #[test]
    fn query_overrides_env_and_project() {
        let r = SecretRef::parse("secret://infisical/a/KEY?env=dev&project=p2").unwrap();
        let m = InfisicalBackend::map_ref(&r, "prod", Some("projX"));
        assert_eq!(m.environment, "dev");
        assert_eq!(m.project_id.as_deref(), Some("p2"));
        assert_eq!(m.secret_name, "KEY");
        assert_eq!(m.secret_path, "/a");
    }

    #[test]
    fn root_level_secret_has_root_path() {
        let r = SecretRef::parse("secret://infisical/JUST_A_KEY").unwrap();
        let m = InfisicalBackend::map_ref(&r, "prod", None);
        assert_eq!(m.secret_name, "JUST_A_KEY");
        assert_eq!(m.secret_path, "/");
    }
}
