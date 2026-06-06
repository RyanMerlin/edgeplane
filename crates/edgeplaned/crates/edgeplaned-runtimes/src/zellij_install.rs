//! Idempotent provisioning for the `edgeplane-zrpc` Zellij plugin.
//!
//! The Zellij session is externally managed (systemd + watchdog), so this is a
//! **one-shot setup step**: write the Zellij config and permissions.kdl the
//! session reads at startup. It is safe to call repeatedly — both operations
//! are idempotent.
//!
//! ## Entry point
//!
//! ```no_run
//! use edgeplaned_runtimes::zellij_install::install_zrpc_plugin;
//!
//! // Resolve the cache dir by running `zellij setup --check` and parsing
//! // the `[CACHE DIR]:` line:
//! let cache_dir = resolve_zellij_cache_dir().unwrap();
//! install_zrpc_plugin(
//!     "/home/user/.config/zellij/config.kdl",   // config_path
//!     &cache_dir,                                 // cache_dir (e.g. /workspace/cache/zellij)
//!     "/opt/edgeplane/edgeplane_zrpc.wasm",      // wasm_path
//! ).unwrap();
//! ```
//!
//! ## CLI wiring
//!
//! A `zrpc install` subcommand in `edgeplane agent` would be the natural
//! surface (resolving `wasm_path` from `EDGEPLANE_ZRPC_PLUGIN_PATH` and
//! `cache_dir` from `resolve_zellij_cache_dir()`). Wiring it into the
//! `commands.rs` `AgentCommand` enum is a deliberate follow-up — the
//! provisioning function is fully exercised through its public API here and
//! does not depend on the CLI being wired.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

// ── Constants ────────────────────────────────────────────────────────────────

/// Alias used in `config.kdl` for the edgeplane-zrpc plugin.
const PLUGIN_ALIAS: &str = "edgeplane-zrpc";

/// Permissions the plugin requires. These are the exact token strings Zellij
/// accepts in `permissions.kdl`; they map onto [`zellij_tile::PermissionType`].
const REQUIRED_PERMS: &[&str] = &[
    "ReadApplicationState",
    "ChangeApplicationState",
    "WriteToStdin",
    "ReadPaneContents",
    "ReadCliPipes",
];

// ── Public API ───────────────────────────────────────────────────────────────

/// Idempotently provision the `edgeplane-zrpc` plugin for headless use.
///
/// 1. Merges `plugins { edgeplane-zrpc location="file:<wasm_path>" }` and
///    `load_plugins { "edgeplane-zrpc" }` into `config_path`.
/// 2. Writes/merges `<cache_dir>/permissions.kdl` granting the plugin the
///    five required permissions (key = raw absolute wasm path, no `file:`
///    prefix — this matches `RunPluginLocation::to_string()` in zellij-tile).
///
/// Both steps are idempotent: existing entries are preserved; the plugin's
/// block is added only if absent, or replaced if already present.
pub fn install_zrpc_plugin(
    config_path: impl AsRef<Path>,
    cache_dir: impl AsRef<Path>,
    wasm_path: impl AsRef<Path>,
) -> Result<()> {
    let wasm_path = wasm_path.as_ref();
    let wasm_abs = wasm_path
        .canonicalize()
        .unwrap_or_else(|_| wasm_path.to_path_buf());
    let wasm_str = wasm_abs
        .to_str()
        .context("wasm_path is not valid UTF-8")?
        .to_string();

    merge_config_kdl(config_path.as_ref(), &wasm_str)
        .with_context(|| format!("failed to update {}", config_path.as_ref().display()))?;

    let perms_path = cache_dir.as_ref().join("permissions.kdl");
    merge_permissions_kdl(&perms_path, &wasm_str)
        .with_context(|| format!("failed to update {}", perms_path.display()))?;

    Ok(())
}

/// Resolve the Zellij cache directory by running `zellij setup --check` and
/// parsing the `[CACHE DIR]:` line.
///
/// Returns `Err` if the binary is not found or the output lacks the marker.
pub fn resolve_zellij_cache_dir() -> Result<PathBuf> {
    let out = std::process::Command::new(crate::zellij_session::zellij_binary())
        .args(["setup", "--check"])
        .env_remove("ZELLIJ")
        .env_remove("ZELLIJ_SESSION_NAME")
        .output()
        .context("failed to run `zellij setup --check`")?;

    // `zellij setup --check` writes its report to stdout and exits 0 on success
    // or 1 if checks fail; we care only about parsing the cache-dir line.
    let combined = String::from_utf8_lossy(&out.stdout).to_string()
        + &String::from_utf8_lossy(&out.stderr);

    for line in combined.lines() {
        // Format: "[CACHE DIR]:        /workspace/cache/zellij"
        if let Some(rest) = line.strip_prefix("[CACHE DIR]:") {
            let dir = rest.trim();
            if !dir.is_empty() {
                return Ok(PathBuf::from(dir));
            }
        }
    }

    anyhow::bail!(
        "`zellij setup --check` output did not contain a `[CACHE DIR]:` line. \
         Full output:\n{}",
        combined
    )
}

// ── config.kdl merge ─────────────────────────────────────────────────────────

/// Merge the plugin alias + load-plugins declaration into `config_path`.
///
/// The Zellij config is KDL, but we use robust string-section matching rather
/// than a full KDL parse (no external KDL-rewrite dep). The two blocks we need:
///
/// ```kdl
/// plugins {
///     edgeplane-zrpc location="file:/abs/path/to/edgeplane_zrpc.wasm"
/// }
/// load_plugins {
///     "edgeplane-zrpc"
/// }
/// ```
///
/// If either block is absent, we append it. If the alias is already present
/// (e.g. pointing at a different wasm path), we replace the location= value.
pub fn merge_config_kdl(config_path: &Path, wasm_path: &str) -> Result<()> {
    let existing = if config_path.exists() {
        std::fs::read_to_string(config_path)
            .with_context(|| format!("reading {}", config_path.display()))?
    } else {
        String::new()
    };

    let updated = apply_config_kdl_merges(&existing, wasm_path);

    if updated != existing {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(config_path, &updated)
            .with_context(|| format!("writing {}", config_path.display()))?;
    }

    Ok(())
}

/// Pure config-merge logic (extracted for unit tests).
pub fn apply_config_kdl_merges(config: &str, wasm_path: &str) -> String {
    let config = merge_plugins_block(config, wasm_path);
    merge_load_plugins_block(&config)
}

/// Ensure `plugins { edgeplane-zrpc location="file:<wasm>" }` is present.
fn merge_plugins_block(config: &str, wasm_path: &str) -> String {
    let alias_line = format!("    {PLUGIN_ALIAS} location=\"file:{wasm_path}\"");

    if let Some(block) = find_block(config, "plugins") {
        // Block exists. Check whether the alias line is inside it.
        let (before_open, inside, tail) = split_block(config, block);
        // Match any line whose trimmed form is `edgeplane-zrpc` or starts with
        // `edgeplane-zrpc ` (i.e. already has a location= value).
        let alias_prefix = format!("{PLUGIN_ALIAS} ");
        if inside.lines().any(|l| {
            let t = l.trim();
            t == PLUGIN_ALIAS || t.starts_with(alias_prefix.as_str())
        }) {
            // Alias present — replace its line in-place (handles stale wasm path).
            // Preserve original line endings by working line-by-line without
            // re-adding `\n` — instead we replace the matching segment.
            let new_inside = replace_line_in_block(inside, |t| {
                t == PLUGIN_ALIAS || t.starts_with(alias_prefix.as_str())
            }, &alias_line);
            format!("{before_open}{{{new_inside}}}{tail}")
        } else {
            // Alias absent — append inside the block before the closing brace.
            // `inside` already ends with `\n` (it's the content before `}`).
            format!("{before_open}{{{inside}{alias_line}\n}}{tail}")
        }
    } else {
        // No plugins block at all — append one.
        let block = format!("\nplugins {{\n{alias_line}\n}}\n");
        format!("{config}{block}")
    }
}

/// Ensure `load_plugins { "edgeplane-zrpc" }` is present.
fn merge_load_plugins_block(config: &str) -> String {
    let entry_line = format!("    \"{PLUGIN_ALIAS}\"");
    let entry_trimmed = format!("\"{PLUGIN_ALIAS}\"");

    if let Some(block) = find_block(config, "load_plugins") {
        let (before_open, inside, tail) = split_block(config, block);
        if inside.lines().any(|l| l.trim() == entry_trimmed.as_str()) {
            // Already present — no change.
            return config.to_string();
        }
        // Append inside the block.
        format!("{before_open}{{{inside}{entry_line}\n}}{tail}")
    } else {
        let block = format!("\nload_plugins {{\n{entry_line}\n}}\n");
        format!("{config}{block}")
    }
}

/// Replace the first line in `block_inside` (the text between `{` and `}`)
/// for which `matches(trimmed_line)` is true, with `replacement`. Preserves
/// all other lines and their original newline sequences exactly.
fn replace_line_in_block(
    inside: &str,
    matches: impl Fn(&str) -> bool,
    replacement: &str,
) -> String {
    let mut out = String::with_capacity(inside.len() + replacement.len());
    let mut replaced = false;
    // Walk bytes to preserve newline sequences exactly (LF, CRLF).
    let mut remaining = inside;
    while !remaining.is_empty() {
        let (line, rest) = if let Some(pos) = remaining.find('\n') {
            (&remaining[..pos], &remaining[pos + 1..])
        } else {
            (remaining, "")
        };
        // Strip optional trailing CR (for CRLF).
        let line_content = line.strip_suffix('\r').unwrap_or(line);
        let nl = if line.ends_with('\r') { "\r\n" } else { "\n" };

        if !replaced && matches(line_content.trim()) {
            out.push_str(replacement);
            out.push_str(nl);
            replaced = true;
        } else {
            out.push_str(line_content);
            if line.ends_with('\r') {
                out.push('\r');
            }
            if !rest.is_empty() || inside.ends_with('\n') {
                out.push('\n');
            }
        }
        remaining = rest;
    }
    out
}

// ── permissions.kdl merge ────────────────────────────────────────────────────

/// Merge the plugin's permission grant into `permissions.kdl`.
///
/// Zellij's permissions.kdl keys plugin grants by the **raw absolute wasm
/// path** (no `file:` prefix) — this matches `RunPluginLocation::to_string()`
/// in zellij-tile. Each permissions.kdl node looks like:
///
/// ```kdl
/// "/abs/path/to/plugin.wasm" {
///     ReadApplicationState
///     ChangeApplicationState
///     WriteToStdin
///     ReadPaneContents
///     ReadCliPipes
/// }
/// ```
///
/// If the file doesn't exist it is created. If the plugin's node is already
/// present, it is replaced in-place (to pick up permission set changes);
/// other plugins' nodes are preserved unchanged.
pub fn merge_permissions_kdl(perms_path: &Path, wasm_path: &str) -> Result<()> {
    let existing = if perms_path.exists() {
        std::fs::read_to_string(perms_path)
            .with_context(|| format!("reading {}", perms_path.display()))?
    } else {
        String::new()
    };

    let updated = apply_permissions_kdl_merge(&existing, wasm_path);

    if let Some(parent) = perms_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(perms_path, &updated)
        .with_context(|| format!("writing {}", perms_path.display()))?;

    Ok(())
}

/// Pure permissions-merge logic (extracted for unit tests).
pub fn apply_permissions_kdl_merge(existing: &str, wasm_path: &str) -> String {
    // Build the canonical grant block for this plugin.
    let perm_lines: String = REQUIRED_PERMS
        .iter()
        .map(|p| format!("    {p}\n"))
        .collect();
    let grant_block = format!("\"{wasm_path}\" {{\n{perm_lines}}}\n");

    // Key used to identify the plugin's existing node (quoted path).
    let node_key = format!("\"{wasm_path}\"");

    if existing.is_empty() {
        return grant_block;
    }

    // Check whether the plugin's node is already present.
    if let Some(block) = find_block(existing, &node_key) {
        // Replace the existing block in-place.
        let (before_open, _inside, tail) = split_block(existing, block);
        format!("{before_open}{{\n{perm_lines}}}{tail}")
    } else {
        // No existing node — append.
        let sep = if existing.ends_with('\n') { "" } else { "\n" };
        format!("{existing}{sep}{grant_block}")
    }
}

// ── KDL block parsing helpers ────────────────────────────────────────────────
//
// Simple string-based helpers that locate a named KDL block and split the
// surrounding text. These avoid pulling in an external KDL crate, which would
// add a non-trivial dependency for a pure string-append operation. The
// trade-off is that they won't handle deeply nested or pathological KDL, but
// Zellij config files and permissions.kdl are flat enough that this is fine.

/// A located block: byte offsets of the `{` and `}` that bound it.
struct BlockSpan {
    /// Byte offset of `{` (the opening brace).
    open_brace: usize,
    /// Byte offset just past `}` (the closing brace).
    close_end: usize,
}

/// Locate the first top-level KDL block whose node starts with `name`.
fn find_block(text: &str, name: &str) -> Option<BlockSpan> {
    // Walk lines, find one that starts with `name` (optionally followed by
    // space or `{`), then scan forward for the matching brace pair.
    let bytes = text.as_bytes();
    let mut line_start = 0usize;
    while line_start < text.len() {
        let line_end = text[line_start..]
            .find('\n')
            .map(|i| line_start + i + 1)
            .unwrap_or(text.len());
        let line = &text[line_start..line_end];
        let trimmed = line.trim();
        if trimmed == name
            || trimmed.starts_with(&format!("{name} "))
            || trimmed.starts_with(&format!("{name}{{"))
        {
            // Found the node line. Look for `{` in this line or on the next line.
            if let Some(rel) = text[line_start..].find('{') {
                let open_pos = line_start + rel;
                // Walk forward counting braces to find the matching `}`.
                let mut depth = 0usize;
                for (i, &b) in bytes[open_pos..].iter().enumerate() {
                    if b == b'{' {
                        depth += 1;
                    } else if b == b'}' {
                        depth -= 1;
                        if depth == 0 {
                            return Some(BlockSpan {
                                open_brace: open_pos,
                                close_end: open_pos + i + 1,
                            });
                        }
                    }
                }
            }
        }
        line_start = line_end;
    }
    None
}

/// Split `text` around a block into three parts:
/// - `before_open`: everything up to (but not including) `{`
/// - `inside`: everything between `{` and `}` (exclusive)
/// - `tail`: everything after `}` (the character after the closing brace)
///
/// Callers reconstruct as `format!("{before_open}{{{inside}}}{tail}")`.
/// Note the explicit `}}` — the caller must close the block itself.
fn split_block(text: &str, block: BlockSpan) -> (&str, &str, &str) {
    let before_open = &text[..block.open_brace];
    // +1 to skip the `{` itself; content starts after the brace.
    let inside_start = block.open_brace + 1;
    // -1 to stop before the `}`.
    let inside_end = block.close_end - 1;
    let inside = &text[inside_start..inside_end];
    // Tail = everything after the `}`.
    let tail = &text[block.close_end..];
    (before_open, inside, tail)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const WASM: &str = "/opt/edgeplane/edgeplane_zrpc.wasm";

    // ── apply_config_kdl_merges ─────────────────────────────────────────

    #[test]
    fn config_fresh_file_adds_both_blocks() {
        let out = apply_config_kdl_merges("", WASM);
        assert!(
            out.contains(&format!("{PLUGIN_ALIAS} location=\"file:{WASM}\"")),
            "plugins block missing: {out}"
        );
        assert!(
            out.contains(&format!("\"{PLUGIN_ALIAS}\"")),
            "load_plugins block missing: {out}"
        );
    }

    #[test]
    fn config_idempotent_when_both_blocks_present() {
        let initial = apply_config_kdl_merges("", WASM);
        let second = apply_config_kdl_merges(&initial, WASM);
        assert_eq!(
            initial, second,
            "second application should be a no-op"
        );
    }

    #[test]
    fn config_only_plugins_block_missing() {
        let config = "load_plugins {\n    \"edgeplane-zrpc\"\n}\n";
        let out = apply_config_kdl_merges(config, WASM);
        assert!(
            out.contains(&format!("{PLUGIN_ALIAS} location=\"file:{WASM}\"")),
            "should have added plugins block: {out}"
        );
        // load_plugins already present — must not duplicate it.
        assert_eq!(
            out.matches("load_plugins").count(),
            1,
            "load_plugins duplicated: {out}"
        );
    }

    #[test]
    fn config_only_load_plugins_block_missing() {
        let config =
            format!("plugins {{\n    {PLUGIN_ALIAS} location=\"file:{WASM}\"\n}}\n");
        let out = apply_config_kdl_merges(&config, WASM);
        assert!(
            out.contains(&format!("\"{PLUGIN_ALIAS}\"")),
            "should have added load_plugins: {out}"
        );
        // plugins block already present — must not duplicate it.
        // Use "^plugins {" anchor logic: count lines that ARE exactly "plugins {"
        // (not "load_plugins {") to avoid the substring match trap.
        let plugins_block_count = out
            .lines()
            .filter(|l| l.trim_start() == "plugins {")
            .count();
        assert_eq!(plugins_block_count, 1, "plugins block duplicated: {out}");
    }

    #[test]
    fn config_updates_stale_wasm_path() {
        let old_wasm = "/old/path/edgeplane_zrpc.wasm";
        let initial = apply_config_kdl_merges("", old_wasm);
        let updated = apply_config_kdl_merges(&initial, WASM);
        assert!(
            updated.contains(&format!("location=\"file:{WASM}\"")),
            "new path missing: {updated}"
        );
        assert!(
            !updated.contains(&format!("location=\"file:{old_wasm}\"")),
            "old path still present: {updated}"
        );
    }

    #[test]
    fn config_preserves_existing_content() {
        let config = "keybinds clear-defaults=true {\n    normal {\n        // ...\n    }\n}\n";
        let out = apply_config_kdl_merges(config, WASM);
        assert!(
            out.contains("keybinds clear-defaults=true"),
            "existing content lost: {out}"
        );
    }

    // ── apply_permissions_kdl_merge ─────────────────────────────────────

    #[test]
    fn permissions_fresh_file_contains_all_perms() {
        let out = apply_permissions_kdl_merge("", WASM);
        assert!(out.contains(&format!("\"{WASM}\"")), "key missing: {out}");
        for perm in REQUIRED_PERMS {
            assert!(out.contains(perm), "perm {perm} missing: {out}");
        }
    }

    #[test]
    fn permissions_idempotent() {
        let initial = apply_permissions_kdl_merge("", WASM);
        let second = apply_permissions_kdl_merge(&initial, WASM);
        assert_eq!(initial, second, "second call should be a no-op");
    }

    #[test]
    fn permissions_preserves_other_plugin_blocks() {
        let other_plugin = "\"/some/other/plugin.wasm\" {\n    ReadApplicationState\n}\n";
        let out = apply_permissions_kdl_merge(other_plugin, WASM);
        assert!(
            out.contains("/some/other/plugin.wasm"),
            "other plugin lost: {out}"
        );
        assert!(out.contains(&format!("\"{WASM}\"")), "our plugin missing: {out}");
    }

    #[test]
    fn permissions_replaces_stale_block_for_same_plugin() {
        // Start with a block that has fewer perms (e.g. missing WriteToStdin).
        let old = format!(
            "\"{WASM}\" {{\n    ReadApplicationState\n}}\n"
        );
        let out = apply_permissions_kdl_merge(&old, WASM);
        for perm in REQUIRED_PERMS {
            assert!(out.contains(perm), "perm {perm} missing after update: {out}");
        }
        // Must not duplicate the key.
        assert_eq!(
            out.matches(WASM).count(),
            1,
            "plugin key duplicated: {out}"
        );
    }

    // ── install_zrpc_plugin (integration: tempfile) ──────────────────────

    #[test]
    fn install_creates_config_and_permissions() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.kdl");
        let cache_dir = dir.path().join("cache");
        let wasm_path = dir.path().join("edgeplane_zrpc.wasm");
        // Create a minimal wasm file so canonicalize() works.
        std::fs::write(&wasm_path, b"wasm").unwrap();

        install_zrpc_plugin(&config_path, &cache_dir, &wasm_path).unwrap();

        let config = std::fs::read_to_string(&config_path).unwrap();
        let wasm_abs = wasm_path.canonicalize().unwrap();
        let wasm_str = wasm_abs.to_str().unwrap();
        assert!(
            config.contains(&format!("location=\"file:{wasm_str}\"")),
            "config missing plugin entry: {config}"
        );
        assert!(
            config.contains(&format!("\"{PLUGIN_ALIAS}\"")),
            "config missing load_plugins entry: {config}"
        );

        let perms = std::fs::read_to_string(cache_dir.join("permissions.kdl")).unwrap();
        assert!(perms.contains(wasm_str), "permissions.kdl missing key: {perms}");
        for perm in REQUIRED_PERMS {
            assert!(perms.contains(perm), "permissions.kdl missing {perm}: {perms}");
        }
    }

    #[test]
    fn install_is_idempotent_on_disk() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.kdl");
        let cache_dir = dir.path().join("cache");
        let wasm_path = dir.path().join("edgeplane_zrpc.wasm");
        std::fs::write(&wasm_path, b"wasm").unwrap();

        install_zrpc_plugin(&config_path, &cache_dir, &wasm_path).unwrap();
        let config_after_first = std::fs::read_to_string(&config_path).unwrap();
        let perms_after_first =
            std::fs::read_to_string(cache_dir.join("permissions.kdl")).unwrap();

        install_zrpc_plugin(&config_path, &cache_dir, &wasm_path).unwrap();
        let config_after_second = std::fs::read_to_string(&config_path).unwrap();
        let perms_after_second =
            std::fs::read_to_string(cache_dir.join("permissions.kdl")).unwrap();

        assert_eq!(
            config_after_first, config_after_second,
            "config.kdl changed on second install"
        );
        assert_eq!(
            perms_after_first, perms_after_second,
            "permissions.kdl changed on second install"
        );
    }
}
