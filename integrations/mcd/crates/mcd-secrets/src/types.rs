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
    /// Fetch from Infisical via the API.
    Infisical {
        secret_name: String,
        project_id: Option<String>,
        environment: String,
        secret_path: String,
    },
}

/// The result of resolving a set of [`CredentialSource`]s into concrete values.
///
/// Each entry maps the `inject_as` key from the source to its resolved value.
#[derive(Debug, Default)]
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
