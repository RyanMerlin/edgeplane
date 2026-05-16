use crate::client::InfisicalClient;
use crate::config::{InfisicalConfig, InfisicalProfileMap};
use crate::error::{Result, SecretsError};
use crate::types::{CredentialKind, CredentialSource, ResolvedCredentials};

/// Resolve a list of credential sources using a multi-profile map.
///
/// `Infisical` sources can optionally specify a profile name via the
/// `profile` field (reserved for future CredentialKind extension).
/// For now, the active profile is selected from the map.
pub async fn resolve_credentials_with_profiles(
    sources: &[CredentialSource],
    profiles: &InfisicalProfileMap,
) -> Result<ResolvedCredentials> {
    let active = profiles.active_profile();
    match active {
        Some(cfg) => resolve_credentials_with_config(sources, cfg).await,
        None => {
            // No active profile — only Literal and Env sources work
            resolve_credentials_with_config(sources, &InfisicalConfig::default()).await
        }
    }
}

/// Resolve a list of credential sources into concrete env-var values.
///
/// - `Literal` sources are used as-is.
/// - `Env` sources read from the current process environment.
/// - `Infisical` sources fetch from Infisical using the provided config.
///
/// The first resolution error is returned immediately (fail-fast).
pub async fn resolve_credentials(
    sources: &[CredentialSource],
    cfg: &InfisicalConfig,
) -> Result<ResolvedCredentials> {
    resolve_credentials_with_config(sources, cfg).await
}

async fn resolve_credentials_with_config(
    sources: &[CredentialSource],
    cfg: &InfisicalConfig,
) -> Result<ResolvedCredentials> {
    let mut env_vars = std::collections::HashMap::new();

    // Lazy-init the client only when an Infisical source is encountered.
    let mut client: Option<InfisicalClient> = None;

    for source in sources {
        let value = match &source.source {
            CredentialKind::Literal { value } => value.clone(),

            CredentialKind::Env { env_var } => {
                std::env::var(env_var)
                    .map_err(|_| SecretsError::EnvVarMissing(env_var.clone()))?
            }

            CredentialKind::Infisical {
                secret_name,
                project_id,
                environment,
                secret_path,
            } => {
                if !cfg.is_configured() {
                    return Err(SecretsError::TokenMissing);
                }
                let c = if let Some(ref c) = client {
                    c
                } else {
                    client = Some(InfisicalClient::new(cfg)?);
                    client.as_ref().unwrap()
                };
                let proj = project_id
                    .as_deref()
                    .or(cfg.default_project_id.as_deref())
                    .unwrap_or("");
                c.fetch_secret(secret_name, proj, environment, secret_path).await?
            }
        };

        env_vars.insert(source.inject_as.clone(), value);
    }

    Ok(ResolvedCredentials { env_vars })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InfisicalConfig;
    use crate::types::CredentialKind;

    fn default_cfg() -> InfisicalConfig {
        InfisicalConfig::default()
    }

    #[tokio::test]
    async fn resolve_literal() {
        let sources = vec![CredentialSource {
            inject_as: "MY_KEY".to_string(),
            source: CredentialKind::Literal {
                value: "hello-world".to_string(),
            },
        }];
        let result = resolve_credentials(&sources, &default_cfg()).await.unwrap();
        assert_eq!(result.env_vars.get("MY_KEY").unwrap(), "hello-world");
    }

    #[tokio::test]
    async fn resolve_env_present() {
        let var = "MC_MESH_TEST_ENV_PRESENT_99";
        std::env::set_var(var, "env-value");
        let sources = vec![CredentialSource {
            inject_as: "INJECTED".to_string(),
            source: CredentialKind::Env {
                env_var: var.to_string(),
            },
        }];
        let result = resolve_credentials(&sources, &default_cfg()).await.unwrap();
        assert_eq!(result.env_vars.get("INJECTED").unwrap(), "env-value");
        std::env::remove_var(var);
    }

    #[tokio::test]
    async fn resolve_env_missing_is_error() {
        let var = "MC_MESH_TEST_ENV_ABSENT_ZZZZZ";
        std::env::remove_var(var);
        let sources = vec![CredentialSource {
            inject_as: "TARGET".to_string(),
            source: CredentialKind::Env {
                env_var: var.to_string(),
            },
        }];
        let err = resolve_credentials(&sources, &default_cfg()).await.unwrap_err();
        assert!(
            matches!(err, SecretsError::EnvVarMissing(_)),
            "expected EnvVarMissing, got: {err}"
        );
        let msg = err.to_string();
        assert!(msg.contains(var), "error should name missing var: {msg}");
    }

    #[tokio::test]
    async fn resolve_infisical_no_token_is_error() {
        let sources = vec![CredentialSource {
            inject_as: "SECRET".to_string(),
            source: CredentialKind::Infisical {
                secret_name: "MY_SECRET".to_string(),
                project_id: None,
                environment: "prod".to_string(),
                secret_path: "/".to_string(),
            },
        }];
        // default config has empty service_token
        let err = resolve_credentials(&sources, &default_cfg()).await.unwrap_err();
        assert!(
            matches!(err, SecretsError::TokenMissing),
            "expected TokenMissing, got: {err}"
        );
    }

    #[tokio::test]
    async fn into_env_pairs_round_trip() {
        let sources = vec![
            CredentialSource {
                inject_as: "B".to_string(),
                source: CredentialKind::Literal {
                    value: "beta".to_string(),
                },
            },
            CredentialSource {
                inject_as: "A".to_string(),
                source: CredentialKind::Literal {
                    value: "alpha".to_string(),
                },
            },
        ];
        let resolved = resolve_credentials(&sources, &default_cfg()).await.unwrap();
        let pairs = resolved.into_env_pairs();
        // into_env_pairs sorts by key
        assert_eq!(pairs[0], ("A".to_string(), "alpha".to_string()));
        assert_eq!(pairs[1], ("B".to_string(), "beta".to_string()));
    }
}
