use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Path where node credentials are stored after a successful registration.
pub const NODE_CREDENTIAL_PATH: &str = "/etc/edgeplane/node.json";

#[derive(Serialize, Deserialize, Clone)]
pub struct NodeCredential {
    pub node_id: String,
    pub node_jwt: String,
    pub tower_url: String,
    pub issued_at: String,
}

/// Register this machine as an Edgeplane node.
///
/// Calls `POST {endpoint}/runtime/nodes/register` with the supplied join
/// token, writes the issued JWT to `NODE_CREDENTIAL_PATH`, and prints a
/// confirmation.
pub async fn run(
    join_token: String,
    endpoint: String,
    node_name: Option<String>,
    trust_tier: Option<String>,
) -> anyhow::Result<()> {
    let endpoint = endpoint.trim_end_matches('/').to_string();
    let node_name = node_name
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "edgeplaned-node".to_string())
        });
    let hostname_str = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| node_name.clone());
    let trust_tier = trust_tier.unwrap_or_else(|| "untrusted".to_string());
    let runtime_version = env!("CARGO_PKG_VERSION").to_string();

    let body = serde_json::json!({
        "node_name": node_name,
        "hostname": hostname_str,
        "trust_tier": trust_tier,
        "runtime_version": runtime_version,
        "bootstrap_token": join_token,
    });

    let url = format!("{endpoint}/runtime/nodes/register");
    tracing::info!("registering node '{}' at {}", node_name, url);

    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("failed to connect to edgeplane-tower")?;

    let status = resp.status();
    let resp_body = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        bail!("registration failed ({status}): {resp_body}");
    }

    let resp_json: serde_json::Value =
        serde_json::from_str(&resp_body).context("invalid JSON from tower")?;

    let node_id = resp_json["id"]
        .as_str()
        .context("response missing 'id'")?
        .to_string();
    let node_jwt = resp_json["node_jwt"]
        .as_str()
        .context("response missing 'node_jwt' — tower may be running an older version")?
        .to_string();

    let cred = NodeCredential {
        node_id: node_id.clone(),
        node_jwt,
        tower_url: endpoint.clone(),
        issued_at: chrono::Utc::now().to_rfc3339(),
    };

    write_credential(&cred)?;

    println!("Node registered: {node_id}");
    println!("Credentials saved to {NODE_CREDENTIAL_PATH}");
    println!("Run `edgeplaned run` to start the daemon.");

    Ok(())
}

fn write_credential(cred: &NodeCredential) -> anyhow::Result<()> {
    let path = PathBuf::from(NODE_CREDENTIAL_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    // Write to a temp file then rename for atomicity.
    let tmp = path.with_extension("tmp");
    let json = serde_json::to_string_pretty(cred)?;
    std::fs::write(&tmp, json)
        .with_context(|| format!("failed to write {}", tmp.display()))?;
    // Restrict to root-only read before moving into place.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("failed to rename credential file to {}", path.display()))?;
    Ok(())
}
