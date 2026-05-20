//! One-time idempotent importer for Aria-style `fleet-profiles.toml`.
//!
//! Phase 1 of the daemon-absorption plan. Reads the manifest, upserts an
//! `AgentRecord` + `AgentLaunchContext` row per profile, runs on every
//! daemon startup. Upsert keyed on `(source="fleet_import", id=<profile_name>)`
//! so re-runs update in place — no double-creates, no marker file needed.
//!
//! The Phase 2 `ZellijHosted` runtime impl will pick up these rows on its
//! first launch attempt; until then the supervisor logs a "no runtime
//! impl" line and skips them.

use anyhow::{Context, Result};
use mcd_core::types::StateDirSpec;
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::config::SessionMode;
use crate::daemon::AgentSpec;
use crate::local_registry::{AgentLaunchContext, AgentRecord, LocalRegistry};

/// Source tag for agents imported from `fleet-profiles.toml`.
pub const SOURCE_FLEET_IMPORT: &str = "fleet_import";

/// One `[[profile]]` block from `fleet-profiles.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub name: String,
    pub zellij_session: String,
    /// systemd `--user` unit name (e.g. `aria-work.service`). Used by
    /// the Phase 5 unit-health loop to monitor + restart.
    pub service: String,
    pub state_dir: String,
}

#[derive(Debug, Deserialize)]
struct ProfilesFile {
    profile: Vec<Profile>,
}

/// Parse a TOML manifest at `path` into a list of profiles.
pub fn load_profiles(path: &Path) -> Result<Vec<Profile>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading fleet profiles manifest: {}", path.display()))?;
    let parsed: ProfilesFile = toml::from_str(&raw)
        .with_context(|| format!("parsing fleet profiles manifest: {}", path.display()))?;
    Ok(parsed.profile)
}

/// Summary returned by `import_into` for the daemon log line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    /// Number of fresh agents created.
    pub created: usize,
    /// Number of existing agents whose fields were updated.
    pub updated: usize,
    /// Total profiles processed.
    pub total: usize,
}

/// Resolve the manifest path from config + env, with sensible default.
///
/// Precedence: env `MCD_FLEET_PROFILES_FILE` > config `fleet_profiles_file`
/// > default `~/code/aria/fleet-profiles.toml`. Returns `None` if the
/// resolved path doesn't exist on disk — missing is not an error.
pub fn resolve_manifest_path(config_setting: Option<&Path>) -> Option<PathBuf> {
    let candidate = std::env::var("MCD_FLEET_PROFILES_FILE")
        .ok()
        .map(PathBuf::from)
        .or_else(|| config_setting.map(PathBuf::from))
        .or_else(|| {
            dirs::home_dir().map(|h| h.join("code/aria/fleet-profiles.toml"))
        });
    match candidate {
        Some(p) if p.exists() => Some(p),
        _ => None,
    }
}

/// Import each profile into the registry as a `ZellijHosted` agent with
/// a matching `AgentLaunchContext`. Upserts by `(source, id)` so re-runs
/// are no-ops on unchanged profiles.
pub fn import_into(registry: &LocalRegistry, profiles: &[Profile]) -> Result<ImportSummary> {
    let mut summary = ImportSummary {
        total: profiles.len(),
        ..Default::default()
    };

    // Snapshot existing agents under our source tag so we can distinguish
    // create vs update for the log line.
    let existing: std::collections::HashSet<String> = registry
        .list_by_source(SOURCE_FLEET_IMPORT)?
        .into_iter()
        .map(|r| r.id)
        .collect();

    for profile in profiles {
        let spec = AgentSpec {
            agent_id: profile.name.clone(),
            mission_id: String::new(),
            runtime_kind: "zellij_hosted".to_string(),
            session_mode: SessionMode::Persistent,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: Default::default(),
        };
        let record = AgentRecord::from_spec(&spec, SOURCE_FLEET_IMPORT);
        registry
            .upsert(&record)
            .with_context(|| format!("upserting agent record for profile '{}'", profile.name))?;

        let ctx = AgentLaunchContext {
            source: SOURCE_FLEET_IMPORT.to_string(),
            agent_id: profile.name.clone(),
            vault_folder: Some(profile.name.clone()),
            state_dir_spec: Some(StateDirSpec::Persistent {
                path: PathBuf::from(&profile.state_dir),
            }),
            zellij_session: Some(profile.zellij_session.clone()),
            // Phase 5: store the systemd unit name so the unit-health
            // loop can supervise it.
            systemd_service: Some(profile.service.clone()),
            // supervise_paused defaults to false; upsert preserves the
            // operator's pause state across re-imports.
            supervise_paused: false,
        };
        registry.upsert_launch_context(&ctx).with_context(|| {
            format!("upserting launch context for profile '{}'", profile.name)
        })?;

        if existing.contains(&profile.name) {
            summary.updated += 1;
        } else {
            summary.created += 1;
        }
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_toml() -> &'static str {
        r#"
[[profile]]
name           = "operator"
zellij_session = "operator"
service        = "aria.service"
state_dir      = "/home/merlin/.claude/profiles/operator"

[[profile]]
name           = "work"
zellij_session = "work"
service        = "aria-work.service"
state_dir      = "/home/merlin/.claude/profiles/work"
"#
    }

    #[test]
    fn parses_fleet_profiles_toml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fleet.toml");
        std::fs::write(&path, sample_toml()).unwrap();
        let profiles = load_profiles(&path).unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].name, "operator");
        assert_eq!(profiles[1].zellij_session, "work");
        assert_eq!(profiles[1].state_dir, "/home/merlin/.claude/profiles/work");
    }

    #[test]
    fn resolve_returns_none_for_missing_file() {
        // Override env to a path that doesn't exist.
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist.toml");
        // SAFETY: tests run single-threaded by default in `cargo test` with
        // the test binary; this env mutation does not race with other tests
        // in this module because they don't read MCD_FLEET_PROFILES_FILE.
        // Use a unique key per test if this assumption ever breaks.
        unsafe { std::env::set_var("MCD_FLEET_PROFILES_FILE", &missing); }
        let resolved = resolve_manifest_path(None);
        unsafe { std::env::remove_var("MCD_FLEET_PROFILES_FILE"); }
        assert!(resolved.is_none());
    }

    #[test]
    fn import_creates_agent_and_launch_context() {
        let dir = TempDir::new().unwrap();
        let registry = LocalRegistry::open(&dir.path().join("registry.db")).unwrap();
        let profiles = vec![Profile {
            name: "operator".into(),
            zellij_session: "operator".into(),
            service: "aria.service".into(),
            state_dir: "/home/merlin/.claude/profiles/operator".into(),
        }];
        let summary = import_into(&registry, &profiles).unwrap();
        assert_eq!(summary.created, 1);
        assert_eq!(summary.updated, 0);
        assert_eq!(summary.total, 1);

        let agents = registry.list_by_source(SOURCE_FLEET_IMPORT).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].runtime_kind, "zellij_hosted");
        assert_eq!(agents[0].supervision_mode, "persistent");

        let ctx = registry
            .get_launch_context(SOURCE_FLEET_IMPORT, "operator")
            .unwrap()
            .unwrap();
        assert_eq!(ctx.vault_folder.as_deref(), Some("operator"));
        assert_eq!(ctx.zellij_session.as_deref(), Some("operator"));
        match ctx.state_dir_spec {
            Some(StateDirSpec::Persistent { path }) => {
                assert_eq!(path, PathBuf::from("/home/merlin/.claude/profiles/operator"));
            }
            other => panic!("expected Persistent, got {other:?}"),
        }
    }

    #[test]
    fn import_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let registry = LocalRegistry::open(&dir.path().join("registry.db")).unwrap();
        let profiles = vec![Profile {
            name: "operator".into(),
            zellij_session: "operator".into(),
            service: "aria.service".into(),
            state_dir: "/home/merlin/.claude/profiles/operator".into(),
        }];
        let _ = import_into(&registry, &profiles).unwrap();
        let second = import_into(&registry, &profiles).unwrap();
        assert_eq!(second.created, 0);
        assert_eq!(second.updated, 1);
        assert_eq!(registry.list_by_source(SOURCE_FLEET_IMPORT).unwrap().len(), 1);
    }

    #[test]
    fn import_updates_state_dir_on_change() {
        let dir = TempDir::new().unwrap();
        let registry = LocalRegistry::open(&dir.path().join("registry.db")).unwrap();
        let mut profiles = vec![Profile {
            name: "work".into(),
            zellij_session: "work".into(),
            service: "aria-work.service".into(),
            state_dir: "/old/path".into(),
        }];
        import_into(&registry, &profiles).unwrap();
        profiles[0].state_dir = "/new/path".into();
        import_into(&registry, &profiles).unwrap();
        let ctx = registry.get_launch_context(SOURCE_FLEET_IMPORT, "work").unwrap().unwrap();
        match ctx.state_dir_spec {
            Some(StateDirSpec::Persistent { path }) => assert_eq!(path, PathBuf::from("/new/path")),
            other => panic!("expected Persistent /new/path, got {other:?}"),
        }
    }
}
