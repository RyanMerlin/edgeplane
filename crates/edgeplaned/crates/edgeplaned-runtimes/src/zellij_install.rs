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

use anyhow::{Context, Result, bail};

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
    // H2 fix: fail explicitly if the wasm file doesn't exist rather than
    // silently falling back to a non-canonical path that won't match
    // Zellij's RunPluginLocation key → plugin loads ungranted → hangs.
    let wasm_path = wasm_path.as_ref();
    let wasm_abs = wasm_path.canonicalize().with_context(|| {
        format!(
            "wasm not found at {}; build/install the artifact before provisioning",
            wasm_path.display()
        )
    })?;
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
/// Returns `Err` if the binary is not found, the output lacks the marker,
/// or `zellij` exits with a non-zero status for a non-check reason.
pub fn resolve_zellij_cache_dir() -> Result<PathBuf> {
    let out = std::process::Command::new(crate::zellij_session::zellij_binary())
        .args(["setup", "--check"])
        .env_remove("ZELLIJ")
        .env_remove("ZELLIJ_SESSION_NAME")
        .output()
        .context("failed to run `zellij setup --check`")?;

    // M4 fix: detect non-check exit codes (e.g. zellij internal error before
    // printing the cache-dir line) and surface a distinct error message.
    // `zellij setup --check` exits 1 if checks fail but still prints the
    // report; any other non-zero exit is genuinely unexpected.
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

    // If the exit code was unexpected (not 0 or 1), surface it explicitly.
    let status_hint = match out.status.code() {
        Some(0) | Some(1) => String::new(),
        Some(n) => format!(" (exit code {n})"),
        None => " (killed by signal)".into(),
    };

    bail!(
        "`zellij setup --check` output did not contain a `[CACHE DIR]:` line{status_hint}. \
         Full output:\n{}",
        combined
    )
}

// ── config.kdl merge ─────────────────────────────────────────────────────────

/// Merge the plugin alias + load-plugins declaration into `config_path`.
///
/// Uses the `kdl` crate (format-preserving KDL parser) to safely handle
/// real Zellij configs that contain comments, keybind blocks with brace-
/// bearing string values (e.g. `bind "x" { WriteChars "}" ; }`), and other
/// constructs that the previous hand-rolled brace-scanner mishandled.
///
/// The two blocks we ensure are present:
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
/// If either block is absent, it is appended. If the alias is already present
/// (e.g. pointing at a different wasm path), we update the `location=` value.
/// Idempotent.
pub fn merge_config_kdl(config_path: &Path, wasm_path: &str) -> Result<()> {
    let existing = if config_path.exists() {
        std::fs::read_to_string(config_path)
            .with_context(|| format!("reading {}", config_path.display()))?
    } else {
        String::new()
    };

    let updated = apply_config_kdl_merges(&existing, wasm_path)
        .with_context(|| format!("parsing KDL in {}", config_path.display()))?;

    if updated != existing {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // H1 fix: atomic write — write to a temp file in the same directory
        // then rename over the target. Prevents partial writes from corrupting
        // the user's live config on crash / power loss.
        let parent = config_path.parent().unwrap_or(Path::new("."));
        let tmp = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("creating temp file in {}", parent.display()))?;
        std::io::Write::write_all(&mut tmp.as_file(), updated.as_bytes())
            .with_context(|| format!("writing temp file for {}", config_path.display()))?;
        tmp.persist(config_path)
            .with_context(|| format!("atomically replacing {}", config_path.display()))?;
    }

    Ok(())
}

/// Pure config-merge logic (extracted for unit tests).
///
/// Returns `Err` only if the existing content is not valid KDL (and non-empty).
pub fn apply_config_kdl_merges(config: &str, wasm_path: &str) -> Result<String> {
    // Parse — use v1-fallback so real Zellij v1 configs (e.g. boolean `true`
    // rather than `#true`) are accepted.
    let mut doc: kdl::KdlDocument = if config.trim().is_empty() {
        kdl::KdlDocument::new()
    } else {
        config
            .parse()
            .with_context(|| "could not parse existing config.kdl as KDL")?
    };

    ensure_plugins_entry(&mut doc, wasm_path);
    ensure_load_plugins_entry(&mut doc);

    Ok(doc.to_string())
}

/// Ensure `plugins { edgeplane-zrpc location="file:<wasm>" }` is present.
///
/// If the `plugins` node doesn't exist, append it. If it exists but the
/// `edgeplane-zrpc` child is absent, add it. If it exists with the alias,
/// update `location=` in-place (handles stale wasm path). Idempotent.
fn ensure_plugins_entry(doc: &mut kdl::KdlDocument, wasm_path: &str) {
    let location_val = format!("file:{wasm_path}");

    if let Some(plugins_node) = doc.get_mut("plugins") {
        let children = plugins_node.ensure_children();

        // Find existing edgeplane-zrpc child.
        if let Some(alias_node) = children.get_mut(PLUGIN_ALIAS) {
            // Replace the location= property using insert(), which replaces
            // the entry whole (including its format/repr) rather than patching
            // just the KdlValue — the latter leaves a stale value_repr that
            // would serialise the old path unchanged.
            alias_node.insert(
                "location",
                kdl::KdlEntry::new_prop("location", location_val.as_str()),
            );
        } else {
            // Alias absent — append a new child node.
            let child = build_plugin_alias_node(wasm_path);
            children.nodes_mut().push(child);
        }
    } else {
        // No plugins block — build and append one.
        let mut plugins_node = kdl::KdlNode::new("plugins");
        let mut children = kdl::KdlDocument::new();
        children.nodes_mut().push(build_plugin_alias_node(wasm_path));
        plugins_node.set_children(children);
        doc.nodes_mut().push(plugins_node);
    }
}

/// Build `edgeplane-zrpc location="file:<wasm>"`.
fn build_plugin_alias_node(wasm_path: &str) -> kdl::KdlNode {
    let mut node = kdl::KdlNode::new(PLUGIN_ALIAS);
    let entry = kdl::KdlEntry::new_prop("location", format!("file:{wasm_path}").as_str());
    node.entries_mut().push(entry);
    node
}

/// Ensure `load_plugins { "edgeplane-zrpc" }` is present.
///
/// If the `load_plugins` node doesn't exist, append it. If it exists but
/// the `"edgeplane-zrpc"` positional argument child is absent, add it.
/// Idempotent.
fn ensure_load_plugins_entry(doc: &mut kdl::KdlDocument) {
    if let Some(lp_node) = doc.get_mut("load_plugins") {
        let children = lp_node.ensure_children();

        // Check whether a child node named "edgeplane-zrpc" (quoted) is present.
        let already_present = children
            .nodes()
            .iter()
            .any(|n| n.name().value() == PLUGIN_ALIAS);

        if !already_present {
            children.nodes_mut().push(kdl::KdlNode::new(PLUGIN_ALIAS));
        }
    } else {
        // No load_plugins block — build and append one.
        let mut lp_node = kdl::KdlNode::new("load_plugins");
        let mut children = kdl::KdlDocument::new();
        children.nodes_mut().push(kdl::KdlNode::new(PLUGIN_ALIAS));
        lp_node.set_children(children);
        doc.nodes_mut().push(lp_node);
    }
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

    let updated = apply_permissions_kdl_merge(&existing, wasm_path)
        .with_context(|| format!("parsing KDL in {}", perms_path.display()))?;

    if let Some(parent) = perms_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // H1 fix: atomic write via temp file + rename.
    let parent = perms_path.parent().unwrap_or(Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("creating temp file in {}", parent.display()))?;
    std::io::Write::write_all(&mut tmp.as_file(), updated.as_bytes())
        .with_context(|| format!("writing temp file for {}", perms_path.display()))?;
    tmp.persist(perms_path)
        .with_context(|| format!("atomically replacing {}", perms_path.display()))?;

    Ok(())
}

/// Pure permissions-merge logic (extracted for unit tests).
///
/// Returns `Err` only if the existing content is not valid KDL (and non-empty).
pub fn apply_permissions_kdl_merge(existing: &str, wasm_path: &str) -> Result<String> {
    let mut doc: kdl::KdlDocument = if existing.trim().is_empty() {
        kdl::KdlDocument::new()
    } else {
        existing
            .parse()
            .with_context(|| "could not parse existing permissions.kdl as KDL")?
    };

    // The node name in permissions.kdl is the raw absolute wasm path (quoted
    // string, no `file:` prefix), matching RunPluginLocation::to_string().
    // We search by .name().value() which returns the unquoted string.
    let nodes = doc.nodes_mut();

    // Remove any existing node for this wasm path (replace-in-place semantics).
    nodes.retain(|n| n.name().value() != wasm_path);

    // Build the canonical grant node and append it.
    nodes.push(build_permissions_node(wasm_path));

    Ok(doc.to_string())
}

/// Build:
/// ```kdl
/// "/abs/path/to/plugin.wasm" {
///     ReadApplicationState
///     ChangeApplicationState
///     WriteToStdin
///     ReadPaneContents
///     ReadCliPipes
/// }
/// ```
fn build_permissions_node(wasm_path: &str) -> kdl::KdlNode {
    let mut node = kdl::KdlNode::new(wasm_path);
    let mut children = kdl::KdlDocument::new();
    for perm in REQUIRED_PERMS {
        children.nodes_mut().push(kdl::KdlNode::new(*perm));
    }
    node.set_children(children);
    node
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
        let out = apply_config_kdl_merges("", WASM).unwrap();
        assert!(
            out.contains(&format!("{PLUGIN_ALIAS} location=")),
            "plugins block missing: {out}"
        );
        assert!(
            out.contains(PLUGIN_ALIAS),
            "load_plugins block missing: {out}"
        );
        // Verify the location value contains the wasm path.
        assert!(out.contains(WASM), "wasm path missing in output: {out}");
    }

    #[test]
    fn config_idempotent_when_both_blocks_present() {
        let initial = apply_config_kdl_merges("", WASM).unwrap();
        let second = apply_config_kdl_merges(&initial, WASM).unwrap();
        assert_eq!(
            initial, second,
            "second application should be a no-op"
        );
    }

    #[test]
    fn config_only_plugins_block_missing() {
        // Start with a config that already has load_plugins but no plugins block.
        // We build this via the merge itself on a fresh doc, then strip plugins.
        let full = apply_config_kdl_merges("", WASM).unwrap();
        // Parse and remove the plugins node to simulate the missing-block case.
        let mut doc: kdl::KdlDocument = full.parse().unwrap();
        doc.nodes_mut().retain(|n| n.name().value() != "plugins");
        let partial = doc.to_string();

        let out = apply_config_kdl_merges(&partial, WASM).unwrap();
        assert!(
            out.contains(&format!("{PLUGIN_ALIAS} location=")),
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
        // Build full, strip load_plugins.
        let full = apply_config_kdl_merges("", WASM).unwrap();
        let mut doc: kdl::KdlDocument = full.parse().unwrap();
        doc.nodes_mut().retain(|n| n.name().value() != "load_plugins");
        let partial = doc.to_string();

        let out = apply_config_kdl_merges(&partial, WASM).unwrap();
        assert!(
            out.contains(PLUGIN_ALIAS),
            "should have added load_plugins: {out}"
        );
        // plugins block already present — must not duplicate it. Parse the
        // merged output and count plugins nodes at the top level.
        let merged_doc: kdl::KdlDocument = out.parse().unwrap();
        let plugins_block_count = merged_doc
            .nodes()
            .iter()
            .filter(|n| n.name().value() == "plugins")
            .count();
        assert_eq!(plugins_block_count, 1, "plugins block duplicated: {out}");
    }

    #[test]
    fn config_updates_stale_wasm_path() {
        let old_wasm = "/old/path/edgeplane_zrpc.wasm";
        let initial = apply_config_kdl_merges("", old_wasm).unwrap();
        let updated = apply_config_kdl_merges(&initial, WASM).unwrap();
        assert!(
            updated.contains(WASM),
            "new path missing: {updated}"
        );
        assert!(
            !updated.contains(old_wasm),
            "old path still present: {updated}"
        );
    }

    #[test]
    fn config_preserves_existing_content() {
        // A real Zellij config with a comment and a keybind block that contains
        // brace-bearing string values — the pathological case for the old
        // hand-rolled brace scanner.
        let config = r#"keybinds clear-defaults=true {
    normal {
        // This is a comment
        bind "Ctrl b" { WriteChars "}" ; }
        bind "x" { WriteChars "{hello}" ; }
    }
}
"#;
        let out = apply_config_kdl_merges(config, WASM).unwrap();

        // Parse the merged output to verify structural correctness — this is
        // the key check: the KDL parser must not have been confused by braces
        // inside string values or by comments.
        let doc: kdl::KdlDocument = out
            .parse()
            .expect("merged config must be valid KDL");

        // keybinds, plugins, load_plugins must all be present.
        let top_names: Vec<_> = doc.nodes().iter().map(|n| n.name().value()).collect();
        assert!(
            top_names.contains(&"keybinds"),
            "keybinds node lost: {top_names:?}"
        );
        assert!(
            top_names.contains(&"plugins"),
            "plugins node missing: {top_names:?}"
        );
        assert!(
            top_names.contains(&"load_plugins"),
            "load_plugins node missing: {top_names:?}"
        );

        // Comment must be preserved through the round-trip (kdl crate is
        // format-preserving for comments).
        assert!(
            out.contains("// This is a comment"),
            "comment lost: {out}"
        );

        // The keybind nodes with brace-bearing string values must survive
        // structurally — check by inspecting the parsed nodes, not the raw
        // string (the serialiser may normalise whitespace around `;`).
        let keybinds_node = doc.get("keybinds").expect("keybinds missing");
        let normal_children = keybinds_node
            .children()
            .and_then(|d| d.get("normal"))
            .and_then(|n| n.children())
            .expect("keybinds > normal children missing");
        let bind_nodes: Vec<_> = normal_children
            .nodes()
            .iter()
            .filter(|n| n.name().value() == "bind")
            .collect();
        assert_eq!(bind_nodes.len(), 2, "expected 2 bind nodes: {out}");

        // Plugin wasm path must appear in the output.
        assert!(out.contains(WASM), "wasm path missing: {out}");
    }

    /// C2/C3/M3 regression: the KDL parser must not miscount braces inside
    /// string values or comments. Verify the merged doc round-trips correctly
    /// through the kdl parser — if the parser corrupted the structure,
    /// re-parsing would fail or produce a different node count.
    #[test]
    fn config_brace_in_string_does_not_corrupt_parse() {
        let config = r#"keybinds {
    normal {
        // comment with { brace }
        bind "x" { WriteChars "}" ; }
    }
}
"#;
        let merged = apply_config_kdl_merges(config, WASM).unwrap();
        // Must re-parse without error.
        let doc: kdl::KdlDocument = merged
            .parse()
            .expect("merged config must be valid KDL");
        // Must have keybinds, plugins, and load_plugins top-level nodes.
        let names: Vec<_> = doc.nodes().iter().map(|n| n.name().value()).collect();
        assert!(names.contains(&"keybinds"), "keybinds node missing: {names:?}");
        assert!(names.contains(&"plugins"), "plugins node missing: {names:?}");
        assert!(names.contains(&"load_plugins"), "load_plugins node missing: {names:?}");
        // Comment must survive the round-trip (kdl is format-preserving for comments).
        assert!(
            merged.contains("// comment with { brace }"),
            "comment with brace lost in round-trip: {merged}"
        );
        // The bind node with a brace-in-string value must survive structurally.
        // We verify via the parsed doc rather than raw string match so the test
        // is not sensitive to the serialiser normalising `;` whitespace.
        let bind_node = doc
            .get("keybinds")
            .and_then(|n| n.children())
            .and_then(|d| d.get("normal"))
            .and_then(|n| n.children())
            .and_then(|d| d.get("bind"))
            .expect("bind node must survive round-trip");
        // The bind node's child WriteChars must have the "}" string argument.
        let wc_node = bind_node
            .children()
            .and_then(|d| d.get("WriteChars"))
            .expect("WriteChars node must survive round-trip");
        // In kdl 4.x, KdlNode::get(index) returns Option<&KdlEntry>; unwrap
        // the entry to compare the underlying KdlValue.
        assert_eq!(
            wc_node.get(0).map(|e| e.value()),
            Some(&kdl::KdlValue::String("}".into())),
            "WriteChars argument must be \"}}\""
        );
    }

    /// Regression test for kdl-6-vs-kdl-4 comment corruption.
    ///
    /// kdl 6 with `v1-fallback` misparses a trailing inline comment that
    /// immediately precedes the next node on the following line.  The corrupted
    /// round-trip output looks like:
    ///
    /// ```text
    /// simplified_ui false
    ///     // keep status-bar verbose pane_frames true   ← next node got eaten
    /// ```
    ///
    /// i.e. `pane_frames true` is silently appended to the comment and
    /// effectively commented out.  With kdl 4.7.1 (native KDL v1) the output
    /// is byte-for-byte identical to the input.
    ///
    /// This test pins that behaviour so any future crate-version bump that
    /// reintroduces the corruption fails immediately.
    #[test]
    fn config_trailing_inline_comment_does_not_eat_next_node() {
        // Mirrors a real Zellij fleet config that triggered the kdl-6 bug.
        // Key elements:
        //  - boolean node with a trailing inline `//` comment on the same line
        //  - the very next line is another bare-boolean node (the one that got
        //    swallowed into the comment under kdl-6)
        //  - a standalone comment-only line
        //  - a keybind block whose string value contains braces (the other
        //    pathological case for the old hand-rolled brace scanner)
        let config = concat!(
            "simplified_ui false    // keep status-bar verbose\n",
            "pane_frames true\n",
            "// standalone comment line\n",
            "mouse_mode true\n",
            "keybinds {\n",
            "    normal {\n",
            "        bind \"Ctrl b\" { WriteChars \"}\" ; }\n",
            "    }\n",
            "}\n",
        );

        let out = apply_config_kdl_merges(config, WASM).unwrap();

        // (a) The trailing inline comment must still appear verbatim.
        assert!(
            out.contains("// keep status-bar verbose"),
            "trailing inline comment was lost or mutated: {out}"
        );

        // (b) `pane_frames true` must exist as its own top-level node — NOT
        //     appended to the comment line or otherwise commented out.
        //     We verify this structurally by re-parsing and checking the node
        //     list, not just by string-searching (the string "pane_frames"
        //     could appear inside a comment and still fool a grep).
        let doc: kdl::KdlDocument = out
            .parse()
            .expect("merged output must be valid KDL");
        let top_names: Vec<_> = doc.nodes().iter().map(|n| n.name().value()).collect();
        assert!(
            top_names.contains(&"pane_frames"),
            "pane_frames was swallowed into the comment (kdl-6 regression): top-level nodes = {top_names:?}\nfull output:\n{out}"
        );
        // Its value must be the boolean `true`.
        let pane_frames_node = doc.get("pane_frames").unwrap();
        assert_eq!(
            pane_frames_node.get(0).map(|e| e.value()),
            Some(&kdl::KdlValue::Bool(true)),
            "pane_frames value was corrupted: {out}"
        );

        // (c) The standalone comment-only line must survive the round-trip.
        assert!(
            out.contains("// standalone comment line"),
            "standalone comment line was lost: {out}"
        );

        // (d) The brace-in-string keybind must survive structurally.
        let wc_node = doc
            .get("keybinds")
            .and_then(|n| n.children())
            .and_then(|d| d.get("normal"))
            .and_then(|n| n.children())
            .and_then(|d| d.get("bind"))
            .and_then(|n| n.children())
            .and_then(|d| d.get("WriteChars"))
            .expect("keybinds > normal > bind > WriteChars must survive");
        assert_eq!(
            wc_node.get(0).map(|e| e.value()),
            Some(&kdl::KdlValue::String("}".into())),
            "brace-in-string WriteChars argument was corrupted: {out}"
        );

        // (e) Both plugin blocks must have been added.
        assert!(
            top_names.contains(&"plugins"),
            "plugins block was not added: {top_names:?}"
        );
        assert!(
            top_names.contains(&"load_plugins"),
            "load_plugins block was not added: {top_names:?}"
        );
        assert!(out.contains(WASM), "wasm path missing from output: {out}");
    }

    // ── apply_permissions_kdl_merge ─────────────────────────────────────

    #[test]
    fn permissions_fresh_file_contains_all_perms() {
        let out = apply_permissions_kdl_merge("", WASM).unwrap();
        assert!(out.contains(WASM), "key missing: {out}");
        for perm in REQUIRED_PERMS {
            assert!(out.contains(perm), "perm {perm} missing: {out}");
        }
    }

    #[test]
    fn permissions_idempotent() {
        let initial = apply_permissions_kdl_merge("", WASM).unwrap();
        let second = apply_permissions_kdl_merge(&initial, WASM).unwrap();
        // The doc-level output may differ slightly in whitespace after
        // round-tripping through the KDL serializer, but the content must be
        // equivalent: same wasm path present, all perms present, no duplicates.
        assert_eq!(
            second.matches(WASM).count(),
            1,
            "wasm path duplicated on second call: {second}"
        );
        for perm in REQUIRED_PERMS {
            assert!(second.contains(perm), "perm {perm} missing on second call: {second}");
        }
    }

    #[test]
    fn permissions_preserves_other_plugin_blocks() {
        let other_plugin = "/some/other/plugin.wasm";
        // Build an initial doc with another plugin's grant.
        let initial = apply_permissions_kdl_merge("", other_plugin).unwrap();
        let out = apply_permissions_kdl_merge(&initial, WASM).unwrap();
        assert!(
            out.contains(other_plugin),
            "other plugin lost: {out}"
        );
        assert!(out.contains(WASM), "our plugin missing: {out}");
    }

    #[test]
    fn permissions_replaces_stale_block_for_same_plugin() {
        // Start with a block that has only one perm (simulating an older install).
        let old = format!(
            "\"{WASM}\" {{\n    ReadApplicationState\n}}\n"
        );
        let out = apply_permissions_kdl_merge(&old, WASM).unwrap();
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
            config.contains(wasm_str),
            "config missing plugin entry: {config}"
        );
        assert!(
            config.contains(PLUGIN_ALIAS),
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

        install_zrpc_plugin(&config_path, &cache_dir, &wasm_path).unwrap();
        let config_after_second = std::fs::read_to_string(&config_path).unwrap();

        assert_eq!(
            config_after_first, config_after_second,
            "config.kdl changed on second install"
        );
        // permissions.kdl is always written (no equality check before write);
        // verify it is still semantically correct.
        let perms = std::fs::read_to_string(cache_dir.join("permissions.kdl")).unwrap();
        let wasm_abs = wasm_path.canonicalize().unwrap();
        let wasm_str = wasm_abs.to_str().unwrap();
        assert_eq!(
            perms.matches(wasm_str).count(),
            1,
            "permissions.kdl has duplicate key after idempotent install"
        );
    }

    /// H2 regression: install_zrpc_plugin must error if the wasm file doesn't
    /// exist rather than silently using a non-canonical path.
    #[test]
    fn install_errors_if_wasm_missing() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.kdl");
        let cache_dir = dir.path().join("cache");
        let wasm_path = dir.path().join("nonexistent.wasm"); // does NOT exist

        let result = install_zrpc_plugin(&config_path, &cache_dir, &wasm_path);
        assert!(result.is_err(), "expected error for missing wasm");
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("wasm not found") || msg.contains("nonexistent.wasm"),
            "error message should mention missing wasm: {msg}"
        );
    }
}
