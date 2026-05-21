//! Idempotent startup bootstrap for per-node coordination state.
//!
//! Called once after fleet_import during daemon startup. Provisions two
//! entities in the MissionControl controlplane (if they don't already exist):
//!
//!   1. `home-{hostname}` mission — per-node coordination inbox (`kind='home'`
//!      at the DB level; the API doesn't expose `kind` in create/list so we
//!      rely on the naming convention to identify home missions).
//!
//!   2. `intake` kluster under that mission — universal landing zone for
//!      unscoped dispatched work; the spawner triages from here.
//!
//! Idempotent: re-running on an already-bootstrapped node logs at DEBUG and
//! returns `Ok(())` without modifying anything.
//!
//! Soft-fail: if the controlplane is unreachable or returns unexpected errors,
//! the function logs a warning and returns `Ok(())` — the daemon continues
//! starting normally. Bootstrap will retry on the next daemon startup.
//!
//! See `docs/design/ephemeral-task-subagents.md` § Decision 5 for the
//! architectural rationale.

use anyhow::Result;
use mcd_core::client::BackendClient;
use serde_json::Value;

/// Summary of what the bootstrap run did — returned for the caller's log line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BootstrapSummary {
    pub mission_created: bool,
    pub kluster_created: bool,
    pub mission_id: String,
    pub kluster_id: String,
}

/// Resolve the node name to use for `home-{hostname}` provisioning.
///
/// Priority:
///   1. `MC_NODE_ID` env var (explicit override — useful in tests and CI)
///   2. Short Tailscale hostname (first label of the FQDN, e.g. "excalibur"
///      from "excalibur.my-tailnet.ts.net")
///   3. OS hostname (`hostname` command / `gethostname`)
pub fn resolve_node_name() -> String {
    // 1. Explicit env override.
    if let Ok(v) = std::env::var("MC_NODE_ID") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    // 2. Tailscale short hostname.
    if let Some(ts_name) = tailscale_short_hostname() {
        return ts_name;
    }

    // 3. OS hostname.
    os_hostname()
}

/// Run the bootstrap sequence against `client`.
///
/// Returns `Ok(BootstrapSummary)` on success. Returns `Ok` with a warning log
/// on any controlplane connectivity or API error (soft-fail).
pub async fn run(client: &BackendClient) -> Result<BootstrapSummary> {
    let node_name = resolve_node_name();
    let mission_name = format!("home-{node_name}");

    tracing::debug!("bootstrap: node_name={node_name}, provisioning mission={mission_name}");

    // Step 1: resolve or create the home mission.
    let (mission_id, mission_created) = match resolve_or_create_mission(client, &mission_name).await {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(
                "bootstrap: could not provision home mission '{mission_name}': {e:#}. \
                 Continuing without bootstrap — will retry on next startup."
            );
            return Ok(BootstrapSummary::default());
        }
    };

    // Step 2: resolve or create the intake kluster under that mission.
    let (kluster_id, kluster_created) =
        match resolve_or_create_intake_kluster(client, &mission_id, &mission_name).await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(
                    "bootstrap: could not provision intake kluster under mission \
                     '{mission_name}' ({mission_id}): {e:#}. \
                     Continuing without bootstrap — will retry on next startup."
                );
                return Ok(BootstrapSummary {
                    mission_created,
                    mission_id,
                    ..Default::default()
                });
            }
        };

    Ok(BootstrapSummary {
        mission_created,
        kluster_created,
        mission_id,
        kluster_id,
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// GET `/missions`, find one with `name == mission_name`. If found, return
/// its id. If not found, POST to create it and return the new id.
async fn resolve_or_create_mission(
    client: &BackendClient,
    mission_name: &str,
) -> Result<(String, bool)> {
    // List all missions and search for one with the right name.
    // The API returns an array of mission objects with at minimum `id` and `name`.
    let missions: Vec<Value> = client
        .get("/missions")
        .await
        .map_err(|e| anyhow::anyhow!("GET /missions failed: {e:#}"))?;

    for m in &missions {
        let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name == mission_name {
            let id = m
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                anyhow::bail!("mission '{mission_name}' found but has no id field");
            }
            tracing::debug!("bootstrap: home mission exists (id={id})");
            return Ok((id, false));
        }
    }

    // Not found — create it.
    tracing::info!("bootstrap: creating home mission '{mission_name}'");
    let body = serde_json::json!({
        "name": mission_name,
        "northstar_md": format!(
            "Per-node coordination inbox for {mission_name}. \
             Unscoped dispatched work lands in the intake kluster; \
             the spawner triages from here to the right destination kluster."
        ),
        "visibility": "private",
        "owners": "",   // defaults to caller's subject on the server side
    });

    let created: Value = client
        .post("/missions", &body)
        .await
        .map_err(|e| anyhow::anyhow!("POST /missions failed: {e:#}"))?;

    let id = created
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("POST /missions response missing 'id' field: {created}"))?
        .to_string();

    tracing::info!("bootstrap: home mission created (id={id})");
    Ok((id, true))
}

/// GET `/missions/{mission_id}/k`, find a kluster named `intake`. If found,
/// return its id. If not found, POST to create it and return the new id.
async fn resolve_or_create_intake_kluster(
    client: &BackendClient,
    mission_id: &str,
    mission_name: &str,
) -> Result<(String, bool)> {
    let path = format!("/missions/{mission_id}/k");
    let klusters: Vec<Value> = client
        .get(&path)
        .await
        .map_err(|e| anyhow::anyhow!("GET {path} failed: {e:#}"))?;

    for k in &klusters {
        let name = k.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name == "intake" {
            let id = k
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                anyhow::bail!("intake kluster found under '{mission_name}' but has no id field");
            }
            tracing::debug!("bootstrap: intake kluster exists under '{mission_name}' (id={id})");
            return Ok((id, false));
        }
    }

    // Not found — create it.
    tracing::info!(
        "bootstrap: creating intake kluster under mission '{mission_name}' ({mission_id})"
    );
    let body = serde_json::json!({
        "name": "intake",
        "owners": "",  // defaults to caller's subject
        "workstream_md": "Universal landing zone for unscoped dispatched work. \
            Spawner triages from here to the right destination kluster via \
            child meshtasks (parent_task_id points at the intake task; \
            intake task gets status 'dispatched'). \
            See docs/design/ephemeral-task-subagents.md § Decision 5.",
    });

    let created: Value = client
        .post(&path, &body)
        .await
        .map_err(|e| anyhow::anyhow!("POST {path} failed: {e:#}"))?;

    let id = created
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!("POST {path} response missing 'id' field: {created}")
        })?
        .to_string();

    tracing::info!(
        "bootstrap: intake kluster created under '{mission_name}' (id={id})"
    );
    Ok((id, true))
}

// ── Node name helpers ─────────────────────────────────────────────────────────

/// Attempt to get the short hostname from Tailscale (first label of FQDN).
fn tailscale_short_hostname() -> Option<String> {
    let out = std::process::Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let fqdn = v
        .get("Self")
        .and_then(|s| s.get("DNSName"))
        .and_then(|n| n.as_str())?;
    // Strip trailing dot; take first label.
    let short = fqdn
        .trim_end_matches('.')
        .split('.')
        .next()?
        .trim();
    if short.is_empty() { None } else { Some(short.to_string()) }
}

/// OS hostname via the `hostname` command (matches `MachineInfo::detect`).
fn os_hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into())
        })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that `resolve_node_name` honours the `MC_NODE_ID` env override.
    #[test]
    fn resolve_node_name_env_override() {
        // SAFETY: single-threaded test runner; no concurrent env access.
        unsafe { std::env::set_var("MC_NODE_ID", "test-node-override"); }
        let name = resolve_node_name();
        unsafe { std::env::remove_var("MC_NODE_ID"); }
        assert_eq!(name, "test-node-override");
    }

    /// Verify that `resolve_node_name` trims whitespace from the env override.
    #[test]
    fn resolve_node_name_env_override_trimmed() {
        unsafe { std::env::set_var("MC_NODE_ID", "  spaced  "); }
        let name = resolve_node_name();
        unsafe { std::env::remove_var("MC_NODE_ID"); }
        assert_eq!(name, "spaced");
    }

    /// Verify that an empty `MC_NODE_ID` falls through to OS hostname.
    #[test]
    fn resolve_node_name_empty_env_falls_through() {
        unsafe { std::env::set_var("MC_NODE_ID", ""); }
        let name = resolve_node_name();
        unsafe { std::env::remove_var("MC_NODE_ID"); }
        // It should be non-empty (either from tailscale or gethostname).
        assert!(!name.is_empty());
        // It should NOT be the empty env value.
        assert_ne!(name, "");
    }

    /// Verify `os_hostname` returns a non-empty string on this machine.
    #[test]
    fn os_hostname_non_empty() {
        let h = os_hostname();
        assert!(!h.is_empty(), "os_hostname() should always return something");
    }

    // ── Idempotency logic tests (mock HTTP via serde_json) ────────────────────
    //
    // We can't easily mock the BackendClient's HTTP layer without a full mock
    // server, so we test the core idempotency logic by constructing the
    // same JSON shapes the API would return and calling the parsing branch
    // inline. The "already exists → no-op" and "not found → POST" branches
    // are the critical invariants.

    #[test]
    fn mission_list_parse_finds_by_name() {
        // Simulate what the API returns for GET /missions.
        let missions = serde_json::json!([
            {"id": "m-old", "name": "some-other-mission"},
            {"id": "m-home", "name": "home-excalibur"},
        ]);
        let arr = missions.as_array().unwrap();
        let target = "home-excalibur";

        let found = arr.iter().find_map(|m| {
            let name = m.get("name").and_then(|n| n.as_str())?;
            if name == target {
                m.get("id").and_then(|i| i.as_str()).map(String::from)
            } else {
                None
            }
        });

        assert_eq!(found.as_deref(), Some("m-home"));
    }

    #[test]
    fn mission_list_parse_returns_none_when_absent() {
        let missions = serde_json::json!([
            {"id": "m-1", "name": "alpha"},
            {"id": "m-2", "name": "beta"},
        ]);
        let arr = missions.as_array().unwrap();
        let target = "home-excalibur";

        let found = arr.iter().find_map(|m| {
            let name = m.get("name").and_then(|n| n.as_str())?;
            if name == target {
                m.get("id").and_then(|i| i.as_str()).map(String::from)
            } else {
                None
            }
        });

        assert!(found.is_none());
    }

    #[test]
    fn kluster_list_parse_finds_intake() {
        let klusters = serde_json::json!([
            {"id": "k-1", "name": "sprint-23"},
            {"id": "k-intake", "name": "intake"},
        ]);
        let arr = klusters.as_array().unwrap();

        let found = arr.iter().find_map(|k| {
            let name = k.get("name").and_then(|n| n.as_str())?;
            if name == "intake" {
                k.get("id").and_then(|i| i.as_str()).map(String::from)
            } else {
                None
            }
        });

        assert_eq!(found.as_deref(), Some("k-intake"));
    }

    #[test]
    fn kluster_list_parse_returns_none_when_absent() {
        let klusters = serde_json::json!([
            {"id": "k-1", "name": "sprint-23"},
        ]);
        let arr = klusters.as_array().unwrap();

        let found = arr.iter().find_map(|k| {
            let name = k.get("name").and_then(|n| n.as_str())?;
            if name == "intake" {
                k.get("id").and_then(|i| i.as_str()).map(String::from)
            } else {
                None
            }
        });

        assert!(found.is_none());
    }

    #[test]
    fn bootstrap_summary_default_is_no_creates() {
        let s = BootstrapSummary::default();
        assert!(!s.mission_created);
        assert!(!s.kluster_created);
        assert!(s.mission_id.is_empty());
        assert!(s.kluster_id.is_empty());
    }
}
