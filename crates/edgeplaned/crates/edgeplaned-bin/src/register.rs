use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct NodeCredential {
    pub node_id: String,
    pub node_jwt: String,
    pub tower_url: String,
    pub issued_at: String,
}

/// Build the registration URL for the given tower endpoint.
///
/// The tower serves the route at `/api/runtime/nodes/register`. Any trailing
/// slash on `endpoint` is stripped so the resulting URL is always clean
/// (e.g. `"http://h:8008"` → `"http://h:8008/api/runtime/nodes/register"`).
pub fn register_url(endpoint: &str) -> String {
    let base = endpoint.trim_end_matches('/');
    format!("{base}/api/runtime/nodes/register")
}

/// Register this machine as an Edgeplane node.
///
/// Calls `POST {endpoint}/api/runtime/nodes/register` with the supplied join
/// token, writes the issued JWT to the edgeplaned config bucket
/// (`$EP_HOME/config/node.json`, default `~/.edgeplane/config/node.json`),
/// and prints a confirmation.
///
/// Note: the bare `endpoint` (without `/api`) is stored in `node.json` as
/// `tower_url`. The daemon's `BackendClient` prepends `/api` automatically via
/// `with_api_prefix("/api")`, so storing the raw base URL prevents a
/// double-prefix (`/api/api/…`) when the daemon reloads the credential.
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

    let url = register_url(&endpoint);
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

    let cred_path = edgeplaned_paths::node_credential_path();
    write_credential(&cred, &cred_path)?;

    println!("Node registered: {node_id}");
    println!("Credentials saved to {}", cred_path.display());
    println!("Run `edgeplaned run` to start the daemon.");

    Ok(())
}

fn write_credential(cred: &NodeCredential, path: &std::path::Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    // Write to a temp file then rename for atomicity. The temp file is created
    // owner-only (0600) from the start so the node JWT is never briefly
    // group/world readable to a same-directory observer.
    let tmp = path.with_extension("tmp");
    let json = serde_json::to_string_pretty(cred)?;
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .with_context(|| format!("failed to write {}", tmp.display()))?;
        f.write_all(json.as_bytes())
            .with_context(|| format!("failed to write {}", tmp.display()))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&tmp, &json)
            .with_context(|| format!("failed to write {}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("failed to rename credential file to {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_url_has_api_prefix() {
        let url = register_url("http://h:8008");
        assert!(
            url.ends_with("/api/runtime/nodes/register"),
            "expected URL to end with /api/runtime/nodes/register, got: {url}"
        );
    }

    #[test]
    fn register_url_strips_trailing_slash() {
        let url = register_url("http://h:8008/");
        assert_eq!(url, "http://h:8008/api/runtime/nodes/register");
        // No double slash after stripping.
        assert!(!url.contains("//api"), "double slash found: {url}");
    }

    #[test]
    fn register_url_no_double_api_prefix() {
        // The stored tower_url (bare base) must not produce /api/api/...
        // This mirrors the daemon contract: BackendClient prepends /api,
        // so tower_url must be the plain base (no /api suffix).
        let url = register_url("http://edgeplane:8008");
        assert!(!url.contains("/api/api"), "double /api prefix found: {url}");
        assert_eq!(url, "http://edgeplane:8008/api/runtime/nodes/register");
    }
}
