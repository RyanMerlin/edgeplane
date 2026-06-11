//! Multi-controlplane connection contexts — `~/.ep/contexts.yaml`.
//!
//! A context bundles a name, base_url, and optional description. One context
//! is "active" at a time. Session tokens are stored per-context at
//! `~/.ep/sessions/<name>.json` (chmod 600).
//!
//! On first use, the existing `~/.ep/config.json` + `~/.ep/session.json` are
//! treated as the "default" context transparently — no files are rewritten
//! until the user explicitly runs `edgeplane context` commands.

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextsFile {
    pub active: String,
    #[serde(default)]
    pub contexts: BTreeMap<String, ContextEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextEntry {
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ── Paths ─────────────────────────────────────────────────────────────────────

pub fn contexts_file_path() -> PathBuf {
    edgeplaned_paths::contexts_path()
}

pub fn sessions_dir() -> PathBuf {
    edgeplaned_paths::sessions_dir()
}

pub fn session_file_for(context_name: &str) -> PathBuf {
    sessions_dir().join(format!("{}.json", context_name))
}

// ── Load / save ───────────────────────────────────────────────────────────────

/// Load contexts. If `contexts.yaml` is absent, derives a "default" context
/// from the legacy `config.json` without writing anything to disk.
pub fn load_contexts() -> ContextsFile {
    let path = contexts_file_path();
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(file) = serde_yaml::from_str::<ContextsFile>(&content) {
                return file;
            }
        }
    }
    migrate_from_legacy()
}

/// Return the active (name, entry) pair. Falls back gracefully if the active
/// name is not found in the map (e.g. after a manual edit).
pub fn active_context(file: &ContextsFile) -> (String, ContextEntry) {
    if let Some(entry) = file.contexts.get(&file.active) {
        return (file.active.clone(), entry.clone());
    }
    // Fallback: first context or localhost
    if let Some((name, entry)) = file.contexts.iter().next() {
        return (name.clone(), entry.clone());
    }
    (
        "default".to_string(),
        ContextEntry { base_url: "http://localhost:8008".to_string(), description: None },
    )
}

pub fn save_contexts(file: &ContextsFile) -> std::io::Result<()> {
    let path = contexts_file_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let yaml = serde_yaml::to_string(file).unwrap_or_default();
    fs::write(&path, yaml)
}

// ── Migration ─────────────────────────────────────────────────────────────────

fn migrate_from_legacy() -> ContextsFile {
    let saved = crate::config::load_saved_config();
    let base_url = saved
        .base_url
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "http://localhost:8008".to_string());

    let mut contexts = BTreeMap::new();
    contexts.insert(
        "default".to_string(),
        ContextEntry { base_url, description: Some("Default context".to_string()) },
    );
    ContextsFile { active: "default".to_string(), contexts }
}
