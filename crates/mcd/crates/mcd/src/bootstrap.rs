//! Idempotent startup bootstrap for operational state.
//!
//! Called once after fleet_import during daemon startup. Ensures two entities
//! exist in the MissionControl controlplane:
//!
//!   1. The default "home" mission (name `home`, overridable via
//!      `MC_HOME_MISSION_NAME` env var). Single global mission — *not* per-node.
//!      It's a regular mission; we deliberately do NOT use the `Mission.kind`
//!      column, which is being soft-deprecated (write-only with no readers).
//!      The name is `home` because that's what the mission IS — the default
//!      home for unscoped operational work and the natural target for any
//!      agent's `home_mission_id`. A deployment can override with whatever
//!      naming convention it prefers via the env var.
//!
//!   2. `intake` kluster under that mission — universal landing zone for
//!      unscoped dispatched work. The spawner (P2) triages from here to the
//!      right destination kluster via child meshtasks with `parent_task_id`
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
//! `kind='home'` model in favor of a single fleet-ops mission.

use anyhow::Result;
use mcd_core::client::BackendClient;
use serde_json::Value;

/// Default name for the fleet operations mission. Overridable via the
/// `MC_HOME_MISSION_NAME` env var if a deployment wants a different name.
pub const DEFAULT_HOME_MISSION_NAME: &str = "home";

/// Name of the intake kluster. Convention, not configurable — the spawner
/// looks for this exact name when triaging unscoped dispatched work.
pub const INTAKE_KLUSTER_NAME: &str = "intake";

/// Summary of what the bootstrap run did — returned for the caller's log line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BootstrapSummary {
    pub mission_created: bool,
    pub kluster_created: bool,
    pub mission_id: String,
    pub kluster_id: String,
}

/// Resolve the home mission name.
///
/// Priority:
///   1. `MC_HOME_MISSION_NAME` env var (deployment override).
///   2. `DEFAULT_HOME_MISSION_NAME` ("home").
pub fn resolve_home_mission_name() -> String {
    if let Ok(v) = std::env::var("MC_HOME_MISSION_NAME") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    DEFAULT_HOME_MISSION_NAME.to_string()
}

/// Run the bootstrap sequence against `client`.
///
/// Returns `Ok(BootstrapSummary)` on success. Returns `Ok` with a warning log
/// on any controlplane connectivity or API error (soft-fail).
pub async fn run(client: &BackendClient) -> Result<BootstrapSummary> {
    let mission_name = resolve_home_mission_name();

    tracing::debug!("bootstrap: ensuring home mission '{mission_name}' + intake kluster");

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
            tracing::debug!("bootstrap: ops mission exists (id={id})");
            return Ok((id, false));
        }
    }

    tracing::info!("bootstrap: creating home mission '{mission_name}'");
    let body = serde_json::json!({
        "name": mission_name,
        "northstar_md": "Default home mission. Holds the intake kluster (landing zone \
            for unscoped dispatched work) and any other operational klusters. \
            Not a strategic workstream — operational scope.",
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
        if name == INTAKE_KLUSTER_NAME {
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

    tracing::info!(
        "bootstrap: creating intake kluster under mission '{mission_name}' ({mission_id})"
    );
    // NB: `workstream_md` is sent but `KlusterCreate` currently doesn't accept it
    // (silently dropped by serde). Track and fix as a separate controlplane API
    // patch; the kluster still gets created correctly.
    let body = serde_json::json!({
        "name": INTAKE_KLUSTER_NAME,
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
        .ok_or_else(|| {
            anyhow::anyhow!("POST {path} response missing 'id' field: {created}")
        })?
        .to_string();

    tracing::info!(
        "bootstrap: intake kluster created under '{mission_name}' (id={id})"
    );
    Ok((id, true))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that `resolve_home_mission_name` honours the env override.
    #[test]
    fn resolve_home_mission_name_env_override() {
        // SAFETY: single-threaded test runner; no concurrent env access.
        unsafe { std::env::set_var("MC_HOME_MISSION_NAME", "fleet-ops-test"); }
        let name = resolve_home_mission_name();
        unsafe { std::env::remove_var("MC_HOME_MISSION_NAME"); }
        assert_eq!(name, "fleet-ops-test");
    }

    /// Verify that whitespace-only env values fall through to the default.
    #[test]
    fn resolve_home_mission_name_whitespace_falls_through() {
        unsafe { std::env::set_var("MC_HOME_MISSION_NAME", "   "); }
        let name = resolve_home_mission_name();
        unsafe { std::env::remove_var("MC_HOME_MISSION_NAME"); }
        assert_eq!(name, DEFAULT_HOME_MISSION_NAME);
    }

    /// Verify that the default name is the documented constant.
    #[test]
    fn default_home_mission_name_is_home() {
        assert_eq!(DEFAULT_HOME_MISSION_NAME, "home");
    }

    /// Parsing: finds mission by exact name match in the list response.
    #[test]
    fn mission_list_parse_finds_by_name() {
        let resp = serde_json::json!([
            {"id": "mission-a", "name": "Some Other Mission"},
            {"id": "mission-b", "name": "home"},
            {"id": "mission-c", "name": "Another"},
        ]);
        let arr = resp.as_array().unwrap();
        let found = arr
            .iter()
            .find(|m| m.get("name").and_then(|n| n.as_str()) == Some("home"))
            .and_then(|m| m.get("id").and_then(|i| i.as_str()));
        assert_eq!(found, Some("mission-b"));
    }

    /// Parsing: returns None when no mission matches.
    #[test]
    fn mission_list_parse_returns_none_when_absent() {
        let resp = serde_json::json!([
            {"id": "mission-a", "name": "Other"},
        ]);
        let arr = resp.as_array().unwrap();
        let found = arr
            .iter()
            .find(|m| m.get("name").and_then(|n| n.as_str()) == Some("home"));
        assert!(found.is_none());
    }

    /// Parsing: finds the intake kluster among others.
    #[test]
    fn kluster_list_parse_finds_intake() {
        let resp = serde_json::json!([
            {"id": "k-other", "name": "research"},
            {"id": "k-intake", "name": "intake"},
        ]);
        let arr = resp.as_array().unwrap();
        let found = arr
            .iter()
            .find(|k| k.get("name").and_then(|n| n.as_str()) == Some(INTAKE_KLUSTER_NAME))
            .and_then(|k| k.get("id").and_then(|i| i.as_str()));
        assert_eq!(found, Some("k-intake"));
    }

    /// Parsing: no intake kluster present.
    #[test]
    fn kluster_list_parse_returns_none_when_absent() {
        let resp = serde_json::json!([
            {"id": "k-other", "name": "research"},
        ]);
        let arr = resp.as_array().unwrap();
        let found = arr
            .iter()
            .find(|k| k.get("name").and_then(|n| n.as_str()) == Some(INTAKE_KLUSTER_NAME));
        assert!(found.is_none());
    }

    /// Default summary represents "no work done."
    #[test]
    fn bootstrap_summary_default_is_no_creates() {
        let s = BootstrapSummary::default();
        assert!(!s.mission_created);
        assert!(!s.kluster_created);
        assert_eq!(s.mission_id, "");
        assert_eq!(s.kluster_id, "");
    }
}
