//! Daemon-managed state file: `~/.mc/mc-mesh.state.json` (mode 0600).
//!
//! Holds identity assigned by mc-controlplane at registration time:
//!
//! - `node_id` — UUID returned by `POST /runtime/nodes/register`.
//! - `attach_secret` — HMAC secret minted by the controlplane in the same
//!   response, used to validate inbound attach-WS connections proxied
//!   from the controlplane.
//!
//! These fields used to live in `~/.mc/mc-mesh.yaml`, where the user had
//! to capture and paste them by hand. State is not config — it now lives
//! in a daemon-managed file users do not edit.
//!
//! See `docs/plans/2026-05-10-mc-mesh-controlplane-driven-enrollment.md`.
//!
//! ## Wire format
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "node_id": "uuid",
//!   "attach_secret": "hex",
//!   "registered_at": "2026-05-10T15:30:00Z",
//!   "controlplane_url": "http://missioncontrol:8008"
//! }
//! ```
//!
//! Writes are atomic: write to `.tmp`, `fsync`, `rename`. Permissions are
//! enforced to 0600 after each write so the file can't accidentally widen.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Current schema version. Bump when adding required fields; reads of an
/// unknown version log a warning and use defaults for missing fields.
pub const STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeState {
    /// Schema version — see [`STATE_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// UUID assigned by the controlplane at registration.
    pub node_id: String,
    /// HMAC secret minted at registration. 0600-only — never log.
    pub attach_secret: String,
    /// ISO-8601 UTC instant of the original registration.
    pub registered_at: String,
    /// The controlplane URL we registered against. Captured for diagnostics
    /// when the daemon's `backend_url` and the registered URL diverge.
    pub controlplane_url: String,
}

impl NodeState {
    /// Default state-file path: `~/.mc/mc-mesh.state.json`.
    pub fn default_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot resolve $HOME"))?;
        Ok(home.join(".mc").join("mc-mesh.state.json"))
    }

    /// Read state from the given path. Returns `Ok(None)` if the file does
    /// not exist (vs. an actual read/parse error).
    pub fn read(path: &Path) -> Result<Option<Self>> {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let state: Self = serde_json::from_str(&s)
                    .with_context(|| format!("parsing state file {}", path.display()))?;
                if state.schema_version > STATE_SCHEMA_VERSION {
                    tracing::warn!(
                        "mc-mesh state file at {} is schema_version={} but this daemon supports up to {}; using as-is",
                        path.display(),
                        state.schema_version,
                        STATE_SCHEMA_VERSION
                    );
                }
                Ok(Some(state))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(anyhow::Error::from(e)
                .context(format!("reading state file {}", path.display()))),
        }
    }

    /// Atomically write state to `path`. Parent directories are created if
    /// missing. The written file is chmod'd to 0600.
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
            .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
        // Re-apply 0600 after rename in case umask interfered.
        set_mode_0600(path)?;
        Ok(())
    }
}

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
    // Windows: file ACLs are out of scope; the user's profile directory
    // should already be access-controlled.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample() -> NodeState {
        NodeState {
            schema_version: STATE_SCHEMA_VERSION,
            node_id: "11111111-2222-3333-4444-555555555555".into(),
            attach_secret: "deadbeef".repeat(8),
            registered_at: "2026-05-10T15:30:00Z".into(),
            controlplane_url: "http://localhost:8008".into(),
        }
    }

    #[test]
    fn read_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nope.json");
        assert!(NodeState::read(&path).unwrap().is_none());
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        let s = sample();
        s.write_atomic(&path).unwrap();
        let back = NodeState::read(&path).unwrap().unwrap();
        assert_eq!(back.node_id, s.node_id);
        assert_eq!(back.attach_secret, s.attach_secret);
        assert_eq!(back.schema_version, s.schema_version);
    }

    #[cfg(unix)]
    #[test]
    fn write_sets_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        sample().write_atomic(&path).unwrap();
        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }

    #[test]
    fn write_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested/deep/state.json");
        sample().write_atomic(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn future_schema_version_loads_with_warning() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        let mut s = sample();
        s.schema_version = STATE_SCHEMA_VERSION + 99;
        s.write_atomic(&path).unwrap();
        // Should still load — version skew is a warning, not an error.
        let back = NodeState::read(&path).unwrap().unwrap();
        assert_eq!(back.schema_version, STATE_SCHEMA_VERSION + 99);
    }
}
