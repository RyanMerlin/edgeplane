use std::collections::BTreeMap;

/// Connection configuration for a single Infisical instance / account.
///
/// Supports two auth mechanisms — exactly one should be set:
/// - **Service token** (`service_token`): legacy single-credential; used as a
///   Bearer token directly.
/// - **Universal Auth** (`client_id` + `client_secret`): client-credentials
///   flow; tokens are cached in `TokenCache` and refreshed automatically.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InfisicalConfig {
    /// Base URL of the Infisical instance.
    pub site_url: String,
    /// Service token (legacy Bearer auth). Mutually exclusive with UA credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_token: Option<String>,
    /// Universal Auth machine-identity client ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Universal Auth machine-identity client secret (stored in OS keyring at runtime).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// Optional default project ID to use when not specified per-call.
    #[serde(default)]
    pub default_project_id: Option<String>,
    /// Default environment slug (e.g. "prod", "dev", "staging").
    #[serde(default = "default_environment")]
    pub default_environment: String,
}

fn default_environment() -> String { "prod".to_string() }

impl Default for InfisicalConfig {
    fn default() -> Self {
        Self {
            site_url: "https://app.infisical.com".to_string(),
            service_token: None,
            client_id: None,
            client_secret: None,
            default_project_id: None,
            default_environment: "prod".to_string(),
        }
    }
}

impl InfisicalConfig {
    /// Returns true if at least one auth credential is present.
    pub fn is_configured(&self) -> bool {
        let has_st = self.service_token.as_deref().is_some_and(|t| !t.trim().is_empty());
        let has_ua = self.client_id.as_deref().is_some_and(|id| !id.trim().is_empty());
        has_st || has_ua
    }

    /// Construct a service-token profile.
    pub fn with_service_token(site_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            site_url: site_url.into(),
            service_token: Some(token.into()),
            ..Self::default()
        }
    }

    /// Construct a Universal Auth profile.
    pub fn with_ua(
        site_url: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        Self {
            site_url: site_url.into(),
            client_id: Some(client_id.into()),
            client_secret: Some(client_secret.into()),
            ..Self::default()
        }
    }
}

// ─── multi-profile map ────────────────────────────────────────────────────────

/// A named collection of Infisical profiles with one active selection.
///
/// Serialises to / from JSON.  Loaded at startup from
/// `~/.ep/infisical_profiles.json`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct InfisicalProfileMap {
    /// All named profiles, sorted by name.
    #[serde(default)]
    pub profiles: BTreeMap<String, InfisicalConfig>,
    /// The currently active profile name.  `None` means no profile is selected.
    #[serde(default)]
    pub active: Option<String>,
}

impl InfisicalProfileMap {
    /// Return the active profile, or `None` if none is set or selected.
    pub fn active_profile(&self) -> Option<&InfisicalConfig> {
        self.active.as_deref().and_then(|name| self.profiles.get(name))
    }

    /// Set the active profile by name.  Returns `Err` if the name is unknown.
    pub fn set_active(&mut self, name: &str) -> Result<(), String> {
        if self.profiles.contains_key(name) {
            self.active = Some(name.to_string());
            Ok(())
        } else {
            Err(format!("profile '{name}' not found"))
        }
    }

    /// Add or overwrite a named profile.
    pub fn upsert(&mut self, name: impl Into<String>, cfg: InfisicalConfig) {
        let name = name.into();
        let is_first = self.profiles.is_empty();
        self.profiles.insert(name.clone(), cfg);
        // Auto-select the first profile added
        if is_first || self.active.is_none() {
            self.active = Some(name);
        }
    }

    /// Remove a profile by name.
    pub fn remove(&mut self, name: &str) -> bool {
        let removed = self.profiles.remove(name).is_some();
        if self.active.as_deref() == Some(name) {
            self.active = self.profiles.keys().next().cloned();
        }
        removed
    }
}

/// Migrate a legacy single-`InfisicalConfig` (from edgeplaned-secrets ≤0.3)
/// into a fresh `InfisicalProfileMap` with the profile named `"default"`.
///
/// Called on first load when no map file exists but a legacy single-config
/// entry may be present in the keyring.
pub fn migrate_legacy(legacy: InfisicalConfig, map: &mut InfisicalProfileMap) {
    if map.profiles.is_empty() && legacy.is_configured() {
        map.upsert("default", legacy);
    }
}
