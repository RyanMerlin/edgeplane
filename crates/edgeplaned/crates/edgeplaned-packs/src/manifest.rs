use serde::{Deserialize, Deserializer, Serialize};
use edgeplaned_sandbox::types::{CgroupLimits, FsPolicy, NetworkPolicy, SideEffectClass};
use edgeplaned_secrets::types::{CredentialKind, CredentialSource};

// ─── Pack manifest (pack.yaml) ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackManifest {
    pub name: String,
    pub version: u32,
    #[serde(default)]
    pub description: Option<String>,
}

// ─── Capability manifest ─────────────────────────────────────────────────────

/// Deserialize sandboxProfile tolerating both string (legacy) and struct (current).
fn deser_sandbox_profile<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<SandboxProfile>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(_) => Ok(None),
        obj => serde_json::from_value(obj)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

fn default_schema() -> serde_json::Value {
    serde_json::json!({"type": "object", "properties": {}})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityManifest {
    pub name: String,
    pub version: u32,
    #[serde(default)]
    pub description: Option<String>,
    pub backend: Backend,
    #[serde(default)]
    pub risk: RiskLevel,
    #[serde(default)]
    pub side_effect_class: SideEffectClass,
    #[serde(default, deserialize_with = "deser_sandbox_profile")]
    pub sandbox_profile: Option<SandboxProfile>,
    #[serde(default = "default_schema")]
    pub input_schema: serde_json::Value,
    /// Credentials resolved from env, literal, or Infisical.
    #[serde(default, deserialize_with = "deser_credentials")]
    pub credentials: Vec<CredentialSource>,
    /// Derived tags (not stored in YAML — populated by the registry loader).
    #[serde(default, skip_deserializing)]
    pub tags: Vec<String>,
}

// ─── SandboxProfile ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxProfile {
    #[serde(default)]
    pub fs: FsPolicy,
    #[serde(default)]
    pub network: NetworkPolicy,
    #[serde(default)]
    pub limits: CgroupLimits,
}

// ─── Backend ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Backend {
    #[serde(rename = "subprocess")]
    Subprocess {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    #[serde(rename = "builtin")]
    Builtin { name: String },
    #[serde(rename = "remote")]
    Remote { url: String },
}

// ─── RiskLevel ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskLevel {
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskLevel::Low => write!(f, "low"),
            RiskLevel::Medium => write!(f, "medium"),
            RiskLevel::High => write!(f, "high"),
            RiskLevel::Critical => write!(f, "critical"),
        }
    }
}

// ─── Credential deserialization (clix YAML → edgeplaned-secrets types) ──────────

/// Clix YAML credential source shape (tagged by `type` field).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ClixCredentialSource {
    #[serde(rename = "env")]
    Env { env_var: String, inject_as: String },
    #[serde(rename = "literal")]
    Literal { value: String, inject_as: String },
    #[serde(rename = "infisical")]
    Infisical {
        secret_name: String,
        #[serde(default)]
        project_id: Option<String>,
        environment: String,
        #[serde(default = "default_secret_path")]
        secret_path: String,
        inject_as: String,
    },
}

fn default_secret_path() -> String {
    "/".to_string()
}

fn deser_credentials<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Vec<CredentialSource>, D::Error> {
    let clix_creds: Vec<ClixCredentialSource> = Vec::deserialize(d)?;
    Ok(clix_creds.into_iter().map(convert_credential).collect())
}

fn convert_credential(c: ClixCredentialSource) -> CredentialSource {
    match c {
        ClixCredentialSource::Env { env_var, inject_as } => CredentialSource {
            inject_as,
            source: CredentialKind::Env { env_var },
        },
        ClixCredentialSource::Literal { value, inject_as } => CredentialSource {
            inject_as,
            source: CredentialKind::Literal { value },
        },
        ClixCredentialSource::Infisical {
            secret_name,
            project_id,
            environment,
            secret_path,
            inject_as,
        } => CredentialSource {
            inject_as,
            source: CredentialKind::Infisical {
                secret_name,
                project_id,
                environment,
                secret_path,
            },
        },
    }
}
