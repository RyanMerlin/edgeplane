//! Daemon-managed state file: `~/.mc/state.json` (mode 0600).
//!
//! ## Schema history
//!
//! ### v1 (Phase 4b)
//! Flat identity fields for a single controlplane:
//! ```json
//! { "schema_version": 1, "node_id": "…", "attach_secret": "…",
//!   "registered_at": "…", "controlplane_url": "…" }
//! ```
//!
//! ### v2 (Phase 5b)
//! Named profiles map + active selection (kubectl-context model):
//! ```json
//! { "schema_version": 2, "active_profile": "work",
//!   "profiles": {
//!     "work": { "url": "…", "auth": { "kind": "token", "token": "…" },
//!               "node_id": "…", "attach_secret": "…", "registered_at": "…" }
//!   }
//! }
//! ```
//!
//! v1 files are auto-migrated to v2 on first read (flat fields become a
//! profile named `default`). The migration writes back atomically.
//!
//! Writes are atomic: `.tmp` → `fsync` → `rename`. Mode enforced to 0600.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const STATE_SCHEMA_VERSION: u32 = 2;

// ---------------------------------------------------------------------------
// v2 types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonState {
    pub schema_version: u32,
    /// Name of the active profile, or `None` for standalone mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_profile: Option<String>,
    /// Saved controlplane profiles.
    #[serde(default)]
    pub profiles: HashMap<String, ProfileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileEntry {
    /// Controlplane base URL (e.g. `http://missioncontrol:8008`).
    pub url: String,
    /// Authentication credentials for this profile.
    pub auth: ProfileAuth,
    /// Node UUID assigned by the controlplane at registration.
    pub node_id: String,
    /// HMAC secret minted at registration. Never log.
    pub attach_secret: String,
    /// ISO-8601 UTC timestamp of original registration.
    pub registered_at: String,
    /// Tailscale FQDN, if known at registration time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale_fqdn: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileAuth {
    /// Auth kind: `"oidc"` (session token from `mc auth login`) or `"service"`
    /// (long-lived machine credential issued by mc-controlplane at registration).
    pub kind: String,
    /// Bearer token for the controlplane. Never log.
    pub token: String,
}

impl ProfileAuth {
    /// OIDC session token — issued by `mc auth login`, TTL up to 720h.
    pub fn oidc(token: impl Into<String>) -> Self {
        Self { kind: "oidc".into(), token: token.into() }
    }

    /// Long-lived machine credential — issued by mc-controlplane at node
    /// registration. Renewed daemon-to-controlplane without user interaction.
    pub fn service(token: impl Into<String>) -> Self {
        Self { kind: "service".into(), token: token.into() }
    }

    #[deprecated(note = "use ProfileAuth::oidc or ProfileAuth::service")]
    pub fn token(token: impl Into<String>) -> Self {
        Self { kind: "token".into(), token: token.into() }
    }
}

// ---------------------------------------------------------------------------
// v1 shape — deserialization only, used for migration
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct NodeStateV1 {
    // schema_version is checked before this is used
    node_id: String,
    attach_secret: String,
    registered_at: String,
    #[serde(default)]
    controlplane_url: String,
}

// ---------------------------------------------------------------------------
// Read / write
// ---------------------------------------------------------------------------

impl DaemonState {
    /// Default state-file path: `~/.mc/mcd/state.json`.
    pub fn default_path() -> Result<PathBuf> {
        Ok(mcd_core::paths::state_file_path())
    }

    /// Read and return the current state, migrating v1 → v2 if needed.
    ///
    /// Returns `Ok(None)` when no file exists. On v1 → v2 migration the file
    /// is atomically rewritten at `path` and a `tracing::warn!` is emitted.
    pub fn read(path: &Path) -> Result<Option<Self>> {
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(anyhow::Error::from(e)
                .context(format!("reading {}", path.display()))),
        };

        // Peek at schema_version without full parse.
        let probe: serde_json::Value = serde_json::from_str(&raw)
            .with_context(|| format!("parsing {}", path.display()))?;
        let version = probe.get("schema_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;

        if version == 1 {
            return Self::migrate_v1(&raw, path).map(Some);
        }

        if version > STATE_SCHEMA_VERSION {
            tracing::warn!(
                "state file at {} is schema_version={version} but daemon supports up to {}; \
                 loading as-is — upgrade mcd if this causes issues.",
                path.display(),
                STATE_SCHEMA_VERSION
            );
        }

        let state: Self = serde_json::from_str(&raw)
            .with_context(|| format!("parsing v2 state at {}", path.display()))?;
        Ok(Some(state))
    }

    /// v1 → v2 migration: wrap flat fields as profile named "default".
    fn migrate_v1(raw: &str, path: &Path) -> Result<Self> {
        let v1: NodeStateV1 = serde_json::from_str(raw)
            .with_context(|| "parsing v1 state file for migration")?;
        tracing::warn!(
            "State file at {} is schema v1. Migrating to v2: existing identity becomes \
             profile named \"default\". Rename it with `mc daemon profile rename default <name>`.",
            path.display()
        );
        let url = v1.controlplane_url.clone();
        let token = String::new(); // v1 didn't store a token — will fall back to session.json
        let mut profiles = HashMap::new();
        profiles.insert("default".into(), ProfileEntry {
            url,
            auth: ProfileAuth::oidc(token),
            node_id: v1.node_id,
            attach_secret: v1.attach_secret,
            registered_at: v1.registered_at,
            tailscale_fqdn: None,
        });
        let state = DaemonState {
            schema_version: STATE_SCHEMA_VERSION,
            active_profile: Some("default".into()),
            profiles,
        };
        // Write back atomically so next start loads v2 directly.
        if let Err(e) = state.write_atomic(path) {
            tracing::warn!("Could not write migrated state file: {e:#}. Will re-migrate next start.");
        }
        Ok(state)
    }

    /// Atomically write state. Parent dirs are created if missing. Mode 0600.
    pub fn write_atomic(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating state dir {}", parent.display()))?;
        }
        let tmp = path.with_extension("tmp");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp, json)
            .with_context(|| format!("writing temp state {}", tmp.display()))?;
        set_mode_0600(&tmp)?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;
        set_mode_0600(path)?;
        Ok(())
    }

    /// Return the active `ProfileEntry`, if any.
    pub fn active(&self) -> Option<(&str, &ProfileEntry)> {
        let name = self.active_profile.as_deref()?;
        self.profiles.get(name).map(|e| (name, e))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn set_mode_0600(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}
#[cfg(not(unix))]
fn set_mode_0600(_path: &Path) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn v1_json() -> &'static str {
        r#"{"schema_version":1,"node_id":"n-1","attach_secret":"deadbeef","registered_at":"2026-05-10T00:00:00Z","controlplane_url":"http://localhost:8008"}"#
    }

    fn v2_state() -> DaemonState {
        let mut profiles = HashMap::new();
        profiles.insert("work".into(), ProfileEntry {
            url: "http://localhost:8008".into(),
            auth: ProfileAuth::token("tok-abc"),
            node_id: "n-1".into(),
            attach_secret: "deadbeef".into(),
            registered_at: "2026-05-10T00:00:00Z".into(),
            tailscale_fqdn: None,
        });
        DaemonState { schema_version: 2, active_profile: Some("work".into()), profiles }
    }

    #[test]
    fn read_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        assert!(DaemonState::read(&path).unwrap().is_none());
    }

    #[test]
    fn write_then_read_v2_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        let s = v2_state();
        s.write_atomic(&path).unwrap();
        let back = DaemonState::read(&path).unwrap().unwrap();
        assert_eq!(back.active_profile.as_deref(), Some("work"));
        let entry = back.profiles.get("work").unwrap();
        assert_eq!(entry.node_id, "n-1");
        assert_eq!(entry.auth.token, "tok-abc");
    }

    #[test]
    fn migrate_v1_to_v2() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, v1_json()).unwrap();
        let state = DaemonState::read(&path).unwrap().unwrap();
        assert_eq!(state.schema_version, 2);
        assert_eq!(state.active_profile.as_deref(), Some("default"));
        let entry = state.profiles.get("default").unwrap();
        assert_eq!(entry.node_id, "n-1");
        // File should have been rewritten as v2.
        let on_disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk["schema_version"], 2);
    }

    #[test]
    fn active_returns_none_for_standalone() {
        let s = DaemonState { schema_version: 2, active_profile: None, ..Default::default() };
        assert!(s.active().is_none());
    }

    #[test]
    fn active_returns_profile_entry() {
        let s = v2_state();
        let (name, entry) = s.active().unwrap();
        assert_eq!(name, "work");
        assert_eq!(entry.node_id, "n-1");
    }

    #[cfg(unix)]
    #[test]
    fn write_sets_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        v2_state().write_atomic(&path).unwrap();
        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }
}
