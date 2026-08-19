//! Idempotent startup bootstrap for operational state.
//!
//! Called once after fleet_import during daemon startup. Ensures two entities
//! exist in the Edgeplane controlplane:
//!
//!   1. The default "home" domain (name `home`, overridable via
//!      `EP_HOME_DOMAIN_NAME` env var). Single global domain — *not* per-node.
//!      It's a regular domain; we deliberately do NOT use the `Domain.kind`
//!      column, which is being soft-deprecated (write-only with no readers).
//!      The name is `home` because that's what the domain IS — the default
//!      home for unscoped operational work and the natural target for any
//!      agent's `home_domain_id`. A deployment can override with whatever
//!      naming convention it prefers via the env var.
//!
//!   2. `intake` mission under that domain — universal landing zone for
//!      unscoped dispatched work. The spawner (P2) triages from here to the
//!      right destination mission via child meshtasks with `parent_task_id`
//!      pointing at the intake task.
//!
//! Idempotent: re-running when both already exist logs at DEBUG and returns
//! `Ok(())` without modifying anything.
//!
//! Soft-fail: if the controlplane is unreachable or returns unexpected errors,
//! the function logs a warning and returns `Ok(())` — the daemon continues
//! starting normally. Bootstrap retries on the next daemon startup.
//!
//! See `docs/design/ephemeral-task-subagents.md` § Decision 5 for the
//! architectural rationale, including why we walked back the per-node
//! `kind='home'` model in favor of a single fleet-ops domain.

use anyhow::Result;
use edgeplaned_core::client::BackendClient;
use serde_json::Value;

/// Default name for the fleet operations domain. Overridable via the
/// `EP_HOME_DOMAIN_NAME` env var if a deployment wants a different name.
pub const DEFAULT_HOME_DOMAIN_NAME: &str = "home";

/// Name of the intake mission. Convention, not configurable — the spawner
/// looks for this exact name when triaging unscoped dispatched work.
pub const INTAKE_MISSION_NAME: &str = "intake";

/// Summary of what the bootstrap run did — returned for the caller's log line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BootstrapSummary {
    pub domain_created: bool,
    pub mission_created: bool,
    pub domain_id: String,
    pub mission_id: String,
}

/// Resolve the home domain name.
///
/// Priority:
///   1. `EP_HOME_DOMAIN_NAME` env var (deployment override).
///   2. `DEFAULT_HOME_DOMAIN_NAME` ("home").
pub fn resolve_home_domain_name() -> String {
    if let Ok(v) = std::env::var("EP_HOME_DOMAIN_NAME") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    DEFAULT_HOME_DOMAIN_NAME.to_string()
}

/// Run the bootstrap sequence against `client`.
///
/// Returns `Ok(BootstrapSummary)` on success. Returns `Ok` with a warning log
/// on any controlplane connectivity or API error (soft-fail).
pub async fn run(client: &BackendClient) -> Result<BootstrapSummary> {
    let domain_name = resolve_home_domain_name();

    tracing::debug!("bootstrap: ensuring home domain '{domain_name}' + intake mission");

    let (domain_id, domain_created) = match resolve_or_create_domain(client, &domain_name).await {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(
                "bootstrap: could not provision home domain '{domain_name}': {e:#}. \
                 Continuing without bootstrap — will retry on next startup."
            );
            return Ok(BootstrapSummary::default());
        }
    };

    let (mission_id, mission_created) =
        match resolve_or_create_intake_mission(client, &domain_id, &domain_name).await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(
                    "bootstrap: could not provision intake mission under domain \
                     '{domain_name}' ({domain_id}): {e:#}. \
                     Continuing without bootstrap — will retry on next startup."
                );
                return Ok(BootstrapSummary {
                    domain_created,
                    domain_id,
                    ..Default::default()
                });
            }
        };

    Ok(BootstrapSummary {
        domain_created,
        mission_created,
        domain_id,
        mission_id,
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// GET `/domains`, find one with `name == domain_name`. If found, return
/// its id. If not found, POST to create it and return the new id.
async fn resolve_or_create_domain(
    client: &BackendClient,
    domain_name: &str,
) -> Result<(String, bool)> {
    let domains: Vec<Value> = client
        .get("/domains")
        .await
        .map_err(|e| anyhow::anyhow!("GET /domains failed: {e:#}"))?;

    for m in &domains {
        let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name == domain_name {
            let id = m
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                anyhow::bail!("domain '{domain_name}' found but has no id field");
            }
            tracing::debug!("bootstrap: ops domain exists (id={id})");
            return Ok((id, false));
        }
    }

    tracing::info!("bootstrap: creating home domain '{domain_name}'");
    let body = serde_json::json!({
        "name": domain_name,
        "northstar_md": "Default home domain. Holds the intake mission (landing zone \
            for unscoped dispatched work) and any other operational missions. \
            Not a strategic workstream — operational scope.",
        "visibility": "private",
        "owners": "",   // defaults to caller's subject on the server side
    });

    let created: Value = client
        .post("/domains", &body)
        .await
        .map_err(|e| anyhow::anyhow!("POST /domains failed: {e:#}"))?;

    let id = created
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("POST /domains response missing 'id' field: {created}"))?
        .to_string();

    tracing::info!("bootstrap: home domain created (id={id})");
    Ok((id, true))
}

/// GET `/domains/{domain_id}/m`, find a mission named `intake`. If found,
/// return its id. If not found, POST to create it and return the new id.
async fn resolve_or_create_intake_mission(
    client: &BackendClient,
    domain_id: &str,
    domain_name: &str,
) -> Result<(String, bool)> {
    let path = format!("/domains/{domain_id}/m");
    let missions: Vec<Value> = client
        .get(&path)
        .await
        .map_err(|e| anyhow::anyhow!("GET {path} failed: {e:#}"))?;

    for k in &missions {
        let name = k.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name == INTAKE_MISSION_NAME {
            let id = k
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                anyhow::bail!("intake mission found under '{domain_name}' but has no id field");
            }
            tracing::debug!("bootstrap: intake mission exists under '{domain_name}' (id={id})");
            return Ok((id, false));
        }
    }

    tracing::info!("bootstrap: creating intake mission under domain '{domain_name}' ({domain_id})");
    // NB: `workstream_md` is sent but `MissionCreate` currently doesn't accept it
    // (silently dropped by serde). Track and fix as a separate controlplane API
    // patch; the mission still gets created correctly.
    let body = serde_json::json!({
        "name": INTAKE_MISSION_NAME,
        "owners": "",
        "workstream_md": "Universal landing zone for unscoped dispatched work. \
            Spawner triages from here via child meshtasks (parent_task_id points \
            back at the intake task; intake task is marked status='dispatched' \
            once routed). See docs/design/ephemeral-task-subagents.md § Decision 5.",
    });

    let created: Value = client
        .post(&path, &body)
        .await
        .map_err(|e| anyhow::anyhow!("POST {path} failed: {e:#}"))?;

    let id = created
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| anyhow::anyhow!("POST {path} response missing 'id' field: {created}"))?
        .to_string();

    tracing::info!("bootstrap: intake mission created under '{domain_name}' (id={id})");
    Ok((id, true))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that `resolve_home_domain_name` honours the env override.
    #[test]
    fn resolve_home_domain_name_env_override() {
        // SAFETY: single-threaded test runner; no concurrent env access.
        unsafe {
            std::env::set_var("EP_HOME_DOMAIN_NAME", "fleet-ops-test");
        }
        let name = resolve_home_domain_name();
        unsafe {
            std::env::remove_var("EP_HOME_DOMAIN_NAME");
        }
        assert_eq!(name, "fleet-ops-test");
    }

    /// Verify that whitespace-only env values fall through to the default.
    #[test]
    fn resolve_home_domain_name_whitespace_falls_through() {
        unsafe {
            std::env::set_var("EP_HOME_DOMAIN_NAME", "   ");
        }
        let name = resolve_home_domain_name();
        unsafe {
            std::env::remove_var("EP_HOME_DOMAIN_NAME");
        }
        assert_eq!(name, DEFAULT_HOME_DOMAIN_NAME);
    }

    /// Verify that the default name is the documented constant.
    #[test]
    fn default_home_domain_name_is_home() {
        assert_eq!(DEFAULT_HOME_DOMAIN_NAME, "home");
    }

    /// Parsing: finds domain by exact name match in the list response.
    #[test]
    fn domain_list_parse_finds_by_name() {
        let resp = serde_json::json!([
            {"id": "domain-a", "name": "Some Other Domain"},
            {"id": "domain-b", "name": "home"},
            {"id": "domain-c", "name": "Another"},
        ]);
        let arr = resp.as_array().unwrap();
        let found = arr
            .iter()
            .find(|m| m.get("name").and_then(|n| n.as_str()) == Some("home"))
            .and_then(|m| m.get("id").and_then(|i| i.as_str()));
        assert_eq!(found, Some("domain-b"));
    }

    /// Parsing: returns None when no domain matches.
    #[test]
    fn domain_list_parse_returns_none_when_absent() {
        let resp = serde_json::json!([
            {"id": "domain-a", "name": "Other"},
        ]);
        let arr = resp.as_array().unwrap();
        let found = arr
            .iter()
            .find(|m| m.get("name").and_then(|n| n.as_str()) == Some("home"));
        assert!(found.is_none());
    }

    /// Parsing: finds the intake mission among others.
    #[test]
    fn mission_list_parse_finds_intake() {
        let resp = serde_json::json!([
            {"id": "k-other", "name": "research"},
            {"id": "k-intake", "name": "intake"},
        ]);
        let arr = resp.as_array().unwrap();
        let found = arr
            .iter()
            .find(|k| k.get("name").and_then(|n| n.as_str()) == Some(INTAKE_MISSION_NAME))
            .and_then(|k| k.get("id").and_then(|i| i.as_str()));
        assert_eq!(found, Some("k-intake"));
    }

    /// Parsing: no intake mission present.
    #[test]
    fn mission_list_parse_returns_none_when_absent() {
        let resp = serde_json::json!([
            {"id": "k-other", "name": "research"},
        ]);
        let arr = resp.as_array().unwrap();
        let found = arr
            .iter()
            .find(|k| k.get("name").and_then(|n| n.as_str()) == Some(INTAKE_MISSION_NAME));
        assert!(found.is_none());
    }

    /// Default summary represents "no work done."
    #[test]
    fn bootstrap_summary_default_is_no_creates() {
        let s = BootstrapSummary::default();
        assert!(!s.domain_created);
        assert!(!s.mission_created);
        assert_eq!(s.domain_id, "");
        assert_eq!(s.mission_id, "");
    }
}
