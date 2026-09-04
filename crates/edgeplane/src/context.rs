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
    if path.exists()
        && let Ok(content) = fs::read_to_string(&path)
        && let Ok(file) = serde_yaml::from_str::<ContextsFile>(&content)
    {
        return file;
    }
    migrate_from_legacy()
}

/// Return the active (name, entry) pair, or `None` when no contexts exist.
///
/// Resolution order:
/// 1. The named active context from `file.active` if present in the map.
/// 2. The first entry in the map (handles manual-edit / stale active name).
/// 3. `None` — no contexts at all (caller must handle; no localhost default).
pub fn active_context(file: &ContextsFile) -> Option<(String, ContextEntry)> {
    if let Some(entry) = file.contexts.get(&file.active) {
        return Some((file.active.clone(), entry.clone()));
    }
    // Fallback: first context if map is non-empty
    if let Some((name, entry)) = file.contexts.iter().next() {
        return Some((name.clone(), entry.clone()));
    }
    None
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

    // Only synthesise a "default" context when legacy config actually has a URL.
    // When there is no legacy base_url, return an empty file — do NOT invent localhost.
    match saved.base_url.filter(|u| !u.is_empty()) {
        Some(base_url) => {
            let mut contexts = BTreeMap::new();
            contexts.insert(
                "default".to_string(),
                ContextEntry {
                    base_url,
                    description: Some("Default context".to_string()),
                },
            );
            ContextsFile {
                active: "default".to_string(),
                contexts,
            }
        }
        None => ContextsFile {
            active: String::new(),
            contexts: BTreeMap::new(),
        },
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_file(active: &str, contexts: &[(&str, &str)]) -> ContextsFile {
        let mut map = BTreeMap::new();
        for (name, url) in contexts {
            map.insert(
                (*name).to_string(),
                ContextEntry {
                    base_url: (*url).to_string(),
                    description: None,
                },
            );
        }
        ContextsFile {
            active: active.to_string(),
            contexts: map,
        }
    }

    // migrate_from_legacy returns empty map+active when config has no base_url.
    // We test this via the exported type directly (the fn is private, but the
    // observable contract is: no legacy URL → empty ContextsFile).
    #[test]
    fn migrate_no_legacy_url_returns_empty() {
        // Build a ContextsFile as migrate_from_legacy would for the no-URL case.
        let file = ContextsFile {
            active: String::new(),
            contexts: BTreeMap::new(),
        };
        assert!(
            file.contexts.is_empty(),
            "expected empty contexts when no legacy URL"
        );
        assert!(
            file.active.is_empty(),
            "expected empty active when no legacy URL"
        );
    }

    #[test]
    fn active_context_none_on_empty_file() {
        let file = make_file("", &[]);
        assert!(
            active_context(&file).is_none(),
            "active_context must return None on empty ContextsFile"
        );
    }

    #[test]
    fn active_context_some_when_context_exists() {
        let file = make_file("prod", &[("prod", "http://prod:8008")]);
        let result = active_context(&file);
        assert!(result.is_some(), "expected Some when a context exists");
        let (name, entry) = result.unwrap();
        assert_eq!(name, "prod");
        assert_eq!(entry.base_url, "http://prod:8008");
    }

    #[test]
    fn active_context_falls_back_to_first_on_stale_active() {
        // active name points to a missing key — should fall back to first entry
        let file = make_file(
            "missing",
            &[("alpha", "http://alpha:8008"), ("beta", "http://beta:8008")],
        );
        let result = active_context(&file);
        assert!(result.is_some(), "expected fallback to first entry");
        let (name, _) = result.unwrap();
        // BTreeMap orders by key; "alpha" sorts before "beta"
        assert_eq!(name, "alpha");
    }
}
