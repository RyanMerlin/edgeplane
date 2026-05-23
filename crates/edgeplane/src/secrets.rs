use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Resolve `secret://...` references.
///
/// Supported providers:
/// - `secret://keychain/<service>/<account>`
/// - `secret://pass/<entry/path>`
/// - `secret://vault/<path>#<field>` (requires `VAULT_ADDR`, `VAULT_TOKEN`)
pub async fn resolve_maybe_secret_ref(value: &str) -> Result<String> {
    if !value.starts_with("secret://") {
        return Ok(value.to_string());
    }
    let parsed = url::Url::parse(value).with_context(|| format!("invalid secret ref: {value}"))?;
    let provider = parsed.host_str().unwrap_or_default();
    match provider {
        "env" => resolve_env(&parsed),
        "keychain" => resolve_keychain(&parsed),
        "pass" => resolve_pass(&parsed),
        "vault" => resolve_vault(&parsed).await,
        "infisical" => resolve_infisical(&parsed),
        _ => Err(anyhow!("unsupported secret provider '{}'", provider)),
    }
}

fn resolve_env(parsed: &url::Url) -> Result<String> {
    let name = parsed.path().trim_start_matches('/');
    if name.is_empty() {
        anyhow::bail!("env ref must be secret://env/<NAME>");
    }
    let value = std::env::var(name).unwrap_or_default();
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("environment variable '{}' is not set", name);
    }
    Ok(trimmed)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecretsProfileConfig {
    pub provider: Option<String>,
    pub infisical_project_id: Option<String>,
    pub infisical_env: Option<String>,
    pub infisical_path: Option<String>,
    #[serde(default)]
    pub refs: BTreeMap<String, String>,
}

pub fn profile_secrets_path(profile_name: &str) -> PathBuf {
    crate::config::mc_home_dir()
        .join("profiles")
        .join(profile_name)
        .join("secrets.json")
}

pub fn load_profile_secrets(profile_name: &str) -> SecretsProfileConfig {
    let path = profile_secrets_path(profile_name);
    let content = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return SecretsProfileConfig::default(),
    };
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn save_profile_secrets(profile_name: &str, cfg: &SecretsProfileConfig) -> Result<()> {
    let path = profile_secrets_path(profile_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(cfg)?)?;
    Ok(())
}

pub fn build_infisical_ref(
    name: &str,
    project_id: Option<&str>,
    env: Option<&str>,
    path: Option<&str>,
) -> String {
    let mut pairs: Vec<(String, String)> = Vec::new();
    if let Some(v) = project_id {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            pairs.push(("projectId".to_string(), trimmed.to_string()));
        }
    }
    if let Some(v) = env {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            pairs.push(("env".to_string(), trimmed.to_string()));
        }
    }
    if let Some(v) = path {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            pairs.push(("path".to_string(), trimmed.to_string()));
        }
    }
    let query = if pairs.is_empty() {
        String::new()
    } else {
        format!(
            "?{}",
            pairs
                .iter()
                .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
                .collect::<Vec<_>>()
                .join("&")
        )
    };
    format!("secret://infisical/{}{}", name.trim(), query)
}

fn resolve_keychain(parsed: &url::Url) -> Result<String> {
    let segments: Vec<&str> = parsed
        .path_segments()
        .map(|s| s.filter(|x| !x.is_empty()).collect())
        .unwrap_or_default();
    if segments.len() < 2 {
        anyhow::bail!("keychain ref must be secret://keychain/<service>/<account>");
    }
    let account = segments.last().unwrap_or(&"").to_string();
    let service = segments[..segments.len() - 1].join("/");
    if service.is_empty() || account.is_empty() {
        anyhow::bail!("keychain ref missing service/account");
    }

    #[cfg(target_os = "macos")]
    {
        let output = Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                &service,
                "-a",
                &account,
                "-w",
            ])
            .output()
            .context("failed to execute security command")?;
        if !output.status.success() {
            anyhow::bail!("keychain secret not found for service/account");
        }
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }

    #[cfg(not(target_os = "macos"))]
    {
        let output = Command::new("secret-tool")
            .args(["lookup", "service", &service, "account", &account])
            .output()
            .context("failed to execute secret-tool")?;
        if !output.status.success() {
            anyhow::bail!("keychain secret not found for service/account");
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

fn resolve_pass(parsed: &url::Url) -> Result<String> {
    let entry = parsed.path().trim_start_matches('/');
    if entry.is_empty() {
        anyhow::bail!("pass ref must be secret://pass/<entry/path>");
    }
    let output = Command::new("pass")
        .args(["show", entry])
        .output()
        .context("failed to execute pass")?;
    if !output.status.success() {
        anyhow::bail!("pass entry not found: {}", entry);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let first = text.lines().next().unwrap_or("").trim().to_string();
    if first.is_empty() {
        anyhow::bail!("pass entry is empty: {}", entry);
    }
    Ok(first)
}

async fn resolve_vault(parsed: &url::Url) -> Result<String> {
    let vault_addr =
        std::env::var("VAULT_ADDR").context("VAULT_ADDR is required for vault refs")?;
    let vault_token =
        std::env::var("VAULT_TOKEN").context("VAULT_TOKEN is required for vault refs")?;
    let path = parsed.path().trim_start_matches('/');
    if path.is_empty() {
        anyhow::bail!("vault ref must be secret://vault/<path>#<field>");
    }
    let field = parsed.fragment().unwrap_or("value");
    let url = format!("{}/v1/{}", vault_addr.trim_end_matches('/'), path);
    let resp: Value = reqwest::Client::new()
        .get(&url)
        .header("X-Vault-Token", vault_token)
        .send()
        .await
        .context("failed to query Vault")?
        .error_for_status()
        .context("vault request failed")?
        .json()
        .await
        .context("vault response was not JSON")?;

    let value = resp
        .get("data")
        .and_then(|d| d.get("data").or(Some(d)))
        .and_then(|d| d.get(field))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if value.is_empty() {
        anyhow::bail!("vault secret field '{}' not found at '{}'", field, path);
    }
    Ok(value)
}

fn resolve_infisical(parsed: &url::Url) -> Result<String> {
    let name = parsed.path().trim_start_matches('/');
    if name.is_empty() {
        anyhow::bail!("infisical ref must be secret://infisical/<name>");
    }
    let mut cmd = Command::new("infisical");
    cmd.args(["secrets", "get", name, "--plain"]);
    for (k, v) in parsed.query_pairs() {
        let key = k.as_ref();
        let value = v.as_ref();
        if value.trim().is_empty() {
            continue;
        }
        match key {
            "projectId" => {
                cmd.arg("--projectId");
                cmd.arg(value);
            }
            "env" => {
                cmd.arg("--env");
                cmd.arg(value);
            }
            "path" => {
                cmd.arg("--path");
                cmd.arg(value);
            }
            _ => {}
        }
    }
    let output = cmd.output().context("failed to execute infisical CLI")?;
    if !output.status.success() {
        anyhow::bail!("infisical secrets get failed for '{}'", name);
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        anyhow::bail!("infisical returned empty value for '{}'", name);
    }
    Ok(text)
}
