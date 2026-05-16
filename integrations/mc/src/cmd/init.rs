/// `mc init --repo` — bootstrap this node from a git-backed sync repo.
///
/// Clones the sync repo to ~/.mc/sync/, stores INFISICAL_TOKEN in
/// the OS keyring (if available and set), and writes ~/.mc/config.json
/// with the `sync_repo` field.
use anyhow::Result;
use std::path::Path;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the `--repo` bootstrap flow.
///
/// `profile` is used as the keyring profile name for the Infisical service token.
/// Pass `None` to default to `"default"`.
pub async fn run_from_repo(repo_url: &str, profile: Option<&str>) -> Result<()> {
    use mcd_sync::SyncClient;

    let profile_name = profile.unwrap_or("default");
    let cache_dir = crate::config::mc_home_dir().join("sync");
    let hostname = resolve_hostname();

    // Step 1: Clone / pull sync repo.
    print!("cloning sync repo {}...", repo_url);
    let client = SyncClient::new(repo_url, &cache_dir, &hostname)?;
    client.pull()?;
    println!(" done");
    println!("✓ sync repo cloned to {}", cache_dir.display());

    // Step 2: Store INFISICAL_TOKEN in keyring.
    store_infisical_token_if_set(profile_name);

    // TODO(mc-init): apply AppArmor profile from sync/fleet/apparmor/ (Phase 2a-3)
    // TODO(mc-init): render goose harness config from sync/fleet/harnesses/goose/
    // TODO(mc-init): register node with MC backend
    // TODO(mc-init): install + enable systemd unit for mcd daemon

    // Step 3: Write config file.
    let config_path = crate::config::mc_home_dir().join("config.json");
    write_init_config(&config_path, repo_url)?;
    println!("✓ config written to {}", config_path.display());

    Ok(())
}

// ---------------------------------------------------------------------------
// Keyring helper
// ---------------------------------------------------------------------------

fn store_infisical_token_if_set(profile_name: &str) {
    match std::env::var("INFISICAL_TOKEN") {
        Ok(token) => {
            #[cfg(target_os = "linux")]
            {
                use mcd_secrets::keyring::{store_service_token, KeyringResult};
                match store_service_token(profile_name, &token) {
                    KeyringResult::Ok => {
                        println!("✓ INFISICAL_TOKEN stored in keyring");
                    }
                    KeyringResult::Unavailable(reason) => {
                        eprintln!(
                            "⚠ keyring unavailable — INFISICAL_TOKEN not persisted: {reason}"
                        );
                    }
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = (profile_name, token);
                // TODO(mc-init): store INFISICAL_TOKEN in keyring via mcd-secrets
                // Keyring support is Linux-only (libsecret / D-Bus secret-service).
                eprintln!("⚠ keyring storage not supported on this platform — INFISICAL_TOKEN not persisted");
            }
        }
        Err(_) => {
            eprintln!("⚠ INFISICAL_TOKEN not set — skipping keyring");
        }
    }
}

// ---------------------------------------------------------------------------
// Config file writing
// ---------------------------------------------------------------------------

/// Write (or merge) `sync_repo` into `~/.mc/config.json`.
///
/// Existing keys in the config are preserved; only `sync_repo` is upserted.
pub fn write_init_config(path: &Path, sync_repo: &str) -> Result<()> {
    let mut config: serde_json::Value = if path.exists() {
        let s = std::fs::read_to_string(path)?;
        serde_json::from_str(&s).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    config["sync_repo"] = serde_json::Value::String(sync_repo.to_string());

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&config)?)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Hostname helper (mirrors cmd/sync.rs)
// ---------------------------------------------------------------------------

fn resolve_hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_init_config_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_init_config(&path, "https://example.com/config.git").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["sync_repo"], "https://example.com/config.git");
    }

    #[test]
    fn write_init_config_merges_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        // Write existing config with a different key.
        std::fs::write(&path, r#"{"capability_route": "local"}"#).unwrap();
        write_init_config(&path, "https://example.com/repo.git").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["sync_repo"], "https://example.com/repo.git");
        // Existing key preserved.
        assert_eq!(v["capability_route"], "local");
    }

    #[test]
    fn write_init_config_overwrites_sync_repo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_init_config(&path, "https://example.com/old.git").unwrap();
        write_init_config(&path, "https://example.com/new.git").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["sync_repo"], "https://example.com/new.git");
    }
}
