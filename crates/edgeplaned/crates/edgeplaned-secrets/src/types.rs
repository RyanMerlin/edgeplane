use std::collections::HashMap;

/// A single credential source that resolves to one env var.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CredentialSource {
    /// The environment variable name to inject the value as.
    pub inject_as: String,
    /// Where the value comes from.
    pub source: CredentialKind,
}

/// The mechanism used to obtain a credential value.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CredentialKind {
    /// A hardcoded literal value.
    Literal { value: String },
    /// Read from a process environment variable at resolution time.
    Env { env_var: String },
    /// Canonical secret reference used by the registry-backed resolver.
    Ref { secret_ref: String },
    /// Fetch from Infisical via the API.
    Infisical {
        secret_name: String,
        #[serde(default)]
        project_id: Option<String>,
        environment: String,
        #[serde(default = "default_secret_path")]
        secret_path: String,
    },
}

fn default_secret_path() -> String {
    "/".to_string()
}

fn contains_any(haystack: &str, forbidden: &[char]) -> bool {
    haystack.chars().any(|c| forbidden.contains(&c))
}

impl CredentialKind {
    /// Returns the canonical `secret://...` reference for ref-like variants.
    pub fn as_secret_ref(&self) -> Option<String> {
        match self {
            CredentialKind::Ref { secret_ref } => Some(secret_ref.clone()),
            CredentialKind::Infisical {
                secret_name,
                project_id,
                environment,
                secret_path,
            } => {
                if contains_any(secret_name, &['/', '?', '#', '&'])
                    || contains_any(environment, &['?', '#', '&'])
                    || contains_any(secret_path, &['?', '#', '&'])
                    || project_id
                        .as_deref()
                        .map(|project_id| contains_any(project_id, &['?', '#', '&']))
                        .unwrap_or(false)
                {
                    return None;
                }
                let dir = secret_path.trim_matches('/');
                let path = if dir.is_empty() {
                    secret_name.clone()
                } else {
                    format!("{dir}/{secret_name}")
                };
                let mut query = format!("env={environment}");
                if let Some(project_id) = project_id.as_deref().filter(|project_id| !project_id.is_empty()) {
                    query.push_str(&format!("&project={project_id}"));
                }
                Some(format!("secret://infisical/{path}?{query}"))
            }
            CredentialKind::Literal { .. } | CredentialKind::Env { .. } => None,
        }
    }
}

/// The result of resolving a set of [`CredentialSource`]s into concrete values.
///
/// Each entry maps the `inject_as` key from the source to its resolved value.
#[derive(Default)]
pub struct ResolvedCredentials {
    pub env_vars: HashMap<String, String>,
}

impl ResolvedCredentials {
    pub fn into_env_pairs(self) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = self.env_vars.into_iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs
    }
}

impl std::fmt::Debug for ResolvedCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(self.env_vars.keys().map(|k| (k, "[REDACTED]")))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_credentials_debug_redacts() {
        let mut env_vars = HashMap::new();
        env_vars.insert("OPENAI_API_KEY".to_string(), "sk-secret".to_string());
        let rc = ResolvedCredentials { env_vars };

        let debug = format!("{rc:?}");
        assert!(debug.contains("OPENAI_API_KEY"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("sk-secret"));
    }

    #[test]
    fn as_secret_ref_rejects_unsafe_components() {
        assert!(
            CredentialKind::Infisical {
                secret_name: "a/b".to_string(),
                project_id: None,
                environment: "prod".to_string(),
                secret_path: "/".to_string(),
            }
            .as_secret_ref()
            .is_none()
        );

        assert!(
            CredentialKind::Infisical {
                secret_name: "KE?Y".to_string(),
                project_id: None,
                environment: "prod".to_string(),
                secret_path: "/".to_string(),
            }
            .as_secret_ref()
            .is_none()
        );

        assert!(
            CredentialKind::Infisical {
                secret_name: "KEY".to_string(),
                project_id: None,
                environment: "prod&x".to_string(),
                secret_path: "/".to_string(),
            }
            .as_secret_ref()
            .is_none()
        );

        assert_eq!(
            CredentialKind::Infisical {
                secret_name: "KEY".to_string(),
                project_id: None,
                environment: "prod".to_string(),
                secret_path: "/a".to_string(),
            }
            .as_secret_ref()
            .as_deref(),
            Some("secret://infisical/a/KEY?env=prod")
        );
    }
}
