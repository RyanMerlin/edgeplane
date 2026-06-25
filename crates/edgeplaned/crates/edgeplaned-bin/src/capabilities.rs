//! Capability vocabulary → `--allowed-tools` translation for ephemeral subagents.
//!
//! # Verified CLI syntax (from `claude --help`)
//!
//! ```text
//! --allowedTools, --allowed-tools <tools...>
//!     Comma or space-separated list of tool names to allow
//!     (e.g. "Bash(git *) Edit")
//! ```
//!
//! This module uses comma-separated output (e.g. `"Bash(ls *),Bash(cat *),Read"`)
//! to match the convention in the design doc. Both separators are accepted by
//! the claude CLI, so either is valid.
//!
//! # Capability vocabulary (v1 — Decision #3)
//!
//! Each coarse capability maps to a set of `--allowed-tools` fragments. Subsuming
//! capabilities (e.g. `vault:write` ⊇ `vault:read`) are expanded to the full
//! union so callers only need to declare the broadest capability they need.
//!
//! | Capability   | Coverage                                    |
//! |--------------|---------------------------------------------|
//! | `shell:read` | Read-only shell commands (ls, cat, grep, …) |
//! | `shell:write`| Full Bash (Bash(*))                         |
//! | `fs:read`    | Read, Glob, Grep                            |
//! | `fs:write`   | Read, Write, Edit, Glob, Grep               |
//! | `vault:read` | vault note read/list/search (via configured vault CLI) |
//! | `vault:write`| vault:read + write/create/patch/append                 |
//! | `edgeplane:read`    | edgeplane agent/daemon read commands               |
//! | `edgeplane:write`   | edgeplane:read + submit/enroll/signal              |
//! | `web:fetch`  | WebFetch, WebSearch                         |
//! | `gh:read`    | gh repo/issue/pr/run view/list, gh api      |
//! | `gh:write`   | gh:read + create/comment                    |
//!
//! See `docs/design/ephemeral-task-subagents.md` Decision #3 for full rationale.

use std::collections::{HashMap, HashSet};

/// A resolved, deduplicated set of `--allowed-tools` fragments.
///
/// Build via [`resolve_capabilities`]; convert to the CLI arg string via
/// [`AllowedTools::to_cli_string`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedTools {
    /// Deduplicated set of individual tool permission fragments.
    fragments: HashSet<String>,
}

impl AllowedTools {
    /// Build from a deduplicated set of fragments (private — use
    /// [`resolve_capabilities`] to construct).
    fn from_set(fragments: HashSet<String>) -> Self {
        Self { fragments }
    }

    /// Return `true` iff no fragments are present (the task declared
    /// `required_capabilities = []`, meaning no tools are allowed).
    ///
    /// Used in tests and may be used by callers inspecting the resolved set
    /// before deciding whether to spawn a subagent.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }

    /// Produce the comma-joined string passed to `claude --allowed-tools`.
    ///
    /// Fragments are sorted for determinism (stable diff, predictable logs).
    pub fn to_cli_string(&self) -> String {
        let mut sorted: Vec<&str> = self.fragments.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        sorted.join(",")
    }
}

// ── Capability vocabulary ─────────────────────────────────────────────────────

/// The canonical map from coarse capability name → set of `--allowed-tools`
/// fragments. Subsuming capabilities include the fragments of the capabilities
/// they subsume (e.g. `vault:write` includes all `vault:read` fragments) so
/// callers that need write access only declare `vault:write`.
fn capability_vocabulary() -> HashMap<&'static str, Vec<&'static str>> {
    let mut m: HashMap<&'static str, Vec<&'static str>> = HashMap::new();

    // shell:read — read-only shell commands; no file modification, no network.
    m.insert(
        "shell:read",
        vec![
            "Bash(ls *)",
            "Bash(cat *)",
            "Bash(head *)",
            "Bash(tail *)",
            "Bash(grep *)",
            "Bash(find *)",
            "Bash(pwd)",
            "Bash(echo *)",
            "Bash(date *)",
        ],
    );

    // shell:write — full Bash; subsumes shell:read implicitly.
    m.insert("shell:write", vec!["Bash(*)"]);

    // fs:read — Claude Code Read/Glob/Grep built-in tools.
    m.insert("fs:read", vec!["Read", "Glob", "Grep"]);

    // fs:write — all fs:read plus Write and Edit.
    m.insert("fs:write", vec!["Read", "Write", "Edit", "Glob", "Grep"]);

    // vault:read — vault note read / list / search via the configured vault CLI.
    m.insert(
        "vault:read",
        vec![
            "Bash(vault note read *)",
            "Bash(vault note list *)",
            "Bash(vault search *)",
        ],
    );

    // vault:write — subsumes vault:read plus mutating operations.
    m.insert(
        "vault:write",
        vec![
            "Bash(vault note read *)",
            "Bash(vault note list *)",
            "Bash(vault search *)",
            "Bash(vault note write *)",
            "Bash(vault note create *)",
            "Bash(vault note patch *)",
            "Bash(vault note append *)",
        ],
    );

    // edgeplane:read — edgeplane CLI status/inspect commands (no state-mutation).
    m.insert(
        "edgeplane:read",
        vec![
            "Bash(edgeplane agent ls *)",
            "Bash(edgeplane daemon agent ls *)",
            "Bash(edgeplane daemon task ls *)",
            "Bash(edgeplane agent cron list)",
            "Bash(edgeplane agent cron describe *)",
            "Bash(edgeplane status *)",
        ],
    );

    // edgeplane:write — subsumes edgeplane:read plus submission / enrollment / signalling.
    m.insert(
        "edgeplane:write",
        vec![
            // Inherited from edgeplane:read
            "Bash(edgeplane agent ls *)",
            "Bash(edgeplane daemon agent ls *)",
            "Bash(edgeplane daemon task ls *)",
            "Bash(edgeplane agent cron list)",
            "Bash(edgeplane agent cron describe *)",
            "Bash(edgeplane status *)",
            // Write operations
            "Bash(edgeplane daemon task submit *)",
            "Bash(edgeplane daemon agent enroll *)",
            "Bash(edgeplane agent signal *)",
        ],
    );

    // web:fetch — Claude Code WebFetch and WebSearch built-in tools.
    m.insert("web:fetch", vec!["WebFetch", "WebSearch"]);

    // gh:read — GitHub CLI and API read operations.
    m.insert(
        "gh:read",
        vec![
            "Bash(gh repo view *)",
            "Bash(gh issue view *)",
            "Bash(gh issue list *)",
            "Bash(gh pr view *)",
            "Bash(gh pr list *)",
            "Bash(gh run list *)",
            "Bash(gh run view *)",
            "Bash(gh api *)",
        ],
    );

    // gh:write — subsumes gh:read plus issue/PR creation and commenting.
    m.insert(
        "gh:write",
        vec![
            // Inherited from gh:read
            "Bash(gh repo view *)",
            "Bash(gh issue view *)",
            "Bash(gh issue list *)",
            "Bash(gh pr view *)",
            "Bash(gh pr list *)",
            "Bash(gh run list *)",
            "Bash(gh run view *)",
            "Bash(gh api *)",
            // Write operations
            "Bash(gh issue create *)",
            "Bash(gh issue comment *)",
            "Bash(gh pr create *)",
            "Bash(gh pr comment *)",
        ],
    );

    m
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Parse the `required_capabilities` TEXT column from a `MeshTask`.
///
/// The column stores a JSON array of strings, e.g. `'["fs:read","shell:write"]'`.
///
/// Returns:
/// - `Ok(vec![])` for an empty string (no capabilities declared).
/// - `Ok(vec![...])` for a valid JSON array of strings.
/// - `Err(...)` for invalid JSON or a non-array JSON value.
pub fn parse_required_capabilities(raw: &str) -> anyhow::Result<Vec<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(vec![]);
    }

    let value: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|e| anyhow::anyhow!("required_capabilities is not valid JSON: {e}"))?;

    match value {
        serde_json::Value::Array(arr) => {
            let mut caps = Vec::with_capacity(arr.len());
            for (i, item) in arr.into_iter().enumerate() {
                match item {
                    serde_json::Value::String(s) => caps.push(s),
                    other => {
                        return Err(anyhow::anyhow!(
                            "required_capabilities[{i}] must be a string, got: {other}"
                        ))
                    }
                }
            }
            Ok(caps)
        }
        other => Err(anyhow::anyhow!(
            "required_capabilities must be a JSON array, got: {}",
            other
        )),
    }
}

/// Translate a list of coarse capability names into a deduplicated
/// [`AllowedTools`] set.
///
/// Returns `Err` if any capability name is not in the v1 vocabulary.
/// An empty input list is valid and produces an empty `AllowedTools`
/// (meaning no tools are allowed — tasks can still produce output via stdout).
pub fn translate_capabilities(
    capabilities: &[String],
) -> anyhow::Result<AllowedTools> {
    let vocab = capability_vocabulary();
    let mut fragments: HashSet<String> = HashSet::new();

    for cap in capabilities {
        match vocab.get(cap.as_str()) {
            Some(frags) => {
                for frag in frags {
                    fragments.insert(frag.to_string());
                }
            }
            None => {
                return Err(anyhow::anyhow!("unknown capability: {cap}"));
            }
        }
    }

    Ok(AllowedTools::from_set(fragments))
}

/// Resolve the final `AllowedTools` for a task, applying strict/lenient mode
/// when `required_capabilities` is empty.
///
/// # Strict mode (`strict = true`)
///
/// Empty or missing capabilities → `Err` with a clear message. The spawner
/// must fail the task immediately. Forces every dispatcher to declare blast radius.
///
/// # Lenient mode (`strict = false`, the default)
///
/// Empty or missing capabilities → attempt to translate `default_capabilities`.
/// If the defaults are invalid (unknown capability name), log a warning and fall
/// back to `["fs:read"]` only. This is the safe-by-default production path.
pub fn resolve_capabilities(
    parsed: &[String],
    strict: bool,
    defaults: &[String],
) -> anyhow::Result<AllowedTools> {
    if !parsed.is_empty() {
        // Explicit capabilities declared — always translate directly.
        return translate_capabilities(parsed);
    }

    if strict {
        return Err(anyhow::anyhow!(
            "task is missing required_capabilities — dispatcher must declare blast radius"
        ));
    }

    // Lenient mode: try the configured defaults, fall back to fs:read only.
    match translate_capabilities(defaults) {
        Ok(tools) => Ok(tools),
        Err(e) => {
            tracing::warn!(
                "capabilities: default_capabilities contains invalid entry ({e:#}). \
                 Falling back to [\"fs:read\"] only."
            );
            translate_capabilities(&["fs:read".to_string()])
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_required_capabilities ───────────────────────────────────────────

    #[test]
    fn parse_required_capabilities_valid_json_array() {
        let raw = r#"["fs:read","shell:write"]"#;
        let result = parse_required_capabilities(raw).unwrap();
        assert_eq!(result, vec!["fs:read".to_string(), "shell:write".to_string()]);
    }

    #[test]
    fn parse_required_capabilities_empty_string_returns_empty_vec() {
        let result = parse_required_capabilities("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_required_capabilities_whitespace_only_returns_empty_vec() {
        let result = parse_required_capabilities("   ").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_required_capabilities_invalid_json_returns_err() {
        let err = parse_required_capabilities("not-json").unwrap_err();
        assert!(err.to_string().contains("not valid JSON"), "got: {err}");
    }

    #[test]
    fn parse_required_capabilities_non_array_returns_err() {
        // A bare JSON string is valid JSON but not an array.
        let err = parse_required_capabilities(r#""not-an-array""#).unwrap_err();
        assert!(
            err.to_string().contains("JSON array"),
            "expected 'JSON array' in error, got: {err}"
        );
    }

    #[test]
    fn parse_required_capabilities_array_with_non_string_element_returns_err() {
        let err = parse_required_capabilities(r#"["fs:read", 42]"#).unwrap_err();
        assert!(
            err.to_string().contains("must be a string"),
            "got: {err}"
        );
    }

    #[test]
    fn parse_required_capabilities_empty_array_returns_empty_vec() {
        let result = parse_required_capabilities("[]").unwrap();
        assert!(result.is_empty());
    }

    // ── translate_capabilities ─────────────────────────────────────────────────

    #[test]
    fn translate_known_capabilities() {
        let caps = vec!["fs:read".to_string(), "shell:write".to_string()];
        let tools = translate_capabilities(&caps).unwrap();
        let cli = tools.to_cli_string();
        // fs:read → Read, Glob, Grep; shell:write → Bash(*)
        assert!(cli.contains("Read"), "missing Read in: {cli}");
        assert!(cli.contains("Glob"), "missing Glob in: {cli}");
        assert!(cli.contains("Grep"), "missing Grep in: {cli}");
        assert!(cli.contains("Bash(*)"), "missing Bash(*) in: {cli}");
    }

    #[test]
    fn translate_unknown_capability_returns_err() {
        let caps = vec!["fs:read".to_string(), "made_up:thing".to_string()];
        let err = translate_capabilities(&caps).unwrap_err();
        assert!(
            err.to_string().contains("unknown capability: made_up:thing"),
            "got: {err}"
        );
    }

    #[test]
    fn translate_subsuming_capability_dedupes() {
        // vault:write already includes all vault:read fragments.
        // Requesting both should produce the same set as vault:write alone.
        let both = translate_capabilities(&[
            "vault:read".to_string(),
            "vault:write".to_string(),
        ])
        .unwrap();
        let write_only =
            translate_capabilities(&["vault:write".to_string()]).unwrap();

        assert_eq!(
            both.fragments, write_only.fragments,
            "vault:read + vault:write should equal vault:write alone"
        );
    }

    #[test]
    fn translate_empty_list_returns_empty_set() {
        let tools = translate_capabilities(&[]).unwrap();
        assert!(tools.is_empty());
        assert_eq!(tools.to_cli_string(), "");
    }

    #[test]
    fn translate_mc_write_includes_mc_read_fragments() {
        let tools = translate_capabilities(&["edgeplane:write".to_string()]).unwrap();
        let cli = tools.to_cli_string();
        // edgeplane:read fragment that should be subsumed
        assert!(cli.contains("Bash(edgeplane agent ls *)"), "missing edgeplane:read fragment in: {cli}");
        // edgeplane:write-specific fragment
        assert!(cli.contains("Bash(edgeplane agent signal *)"), "missing edgeplane:write fragment in: {cli}");
    }

    #[test]
    fn translate_gh_write_includes_gh_read_fragments() {
        let tools = translate_capabilities(&["gh:write".to_string()]).unwrap();
        let cli = tools.to_cli_string();
        assert!(cli.contains("Bash(gh issue list *)"), "missing gh:read fragment in: {cli}");
        assert!(cli.contains("Bash(gh pr create *)"), "missing gh:write fragment in: {cli}");
    }

    // ── build_allowed_tools_flag_joins_with_commas ─────────────────────────────

    #[test]
    fn build_allowed_tools_flag_joins_with_commas() {
        // The to_cli_string output must be comma-joined (no spaces, no other separator).
        let tools = translate_capabilities(&["fs:read".to_string()]).unwrap();
        let cli = tools.to_cli_string();
        // fs:read = Read, Glob, Grep → 3 fragments, sorted alphabetically.
        // Sorted: Glob, Grep, Read → "Glob,Grep,Read"
        assert_eq!(cli, "Glob,Grep,Read", "unexpected joined string: {cli}");
    }

    // ── resolve_capabilities ───────────────────────────────────────────────────

    #[test]
    fn resolve_capabilities_strict_mode_empty_fails() {
        let err = resolve_capabilities(&[], true, &[]).unwrap_err();
        assert!(
            err.to_string().contains("missing required_capabilities"),
            "got: {err}"
        );
    }

    #[test]
    fn resolve_capabilities_lenient_mode_empty_uses_default() {
        let defaults = vec!["fs:read".to_string(), "shell:read".to_string()];
        let tools = resolve_capabilities(&[], false, &defaults).unwrap();
        let cli = tools.to_cli_string();
        // Should include both fs:read (Read,Glob,Grep) and shell:read fragments.
        assert!(cli.contains("Read"), "missing Read in: {cli}");
        assert!(cli.contains("Bash(ls *)"), "missing Bash(ls *) in: {cli}");
    }

    #[test]
    fn resolve_capabilities_lenient_mode_invalid_default_falls_back() {
        // If defaults contain an unknown capability, fall back to fs:read only.
        let bad_defaults = vec!["not:a:real:cap".to_string()];
        let tools = resolve_capabilities(&[], false, &bad_defaults).unwrap();
        let cli = tools.to_cli_string();
        // fs:read fragments only
        assert!(cli.contains("Read"), "missing Read fallback in: {cli}");
        assert!(cli.contains("Glob"), "missing Glob fallback in: {cli}");
        // Must NOT include any shell fragments
        assert!(!cli.contains("Bash("), "unexpected Bash fragment in fallback: {cli}");
    }

    #[test]
    fn resolve_capabilities_with_explicit_capabilities_ignores_defaults() {
        // When explicit capabilities are given, defaults are not used even in lenient mode.
        let parsed = vec!["web:fetch".to_string()];
        let defaults = vec!["fs:read".to_string()];
        let tools = resolve_capabilities(&parsed, false, &defaults).unwrap();
        let cli = tools.to_cli_string();
        assert!(cli.contains("WebFetch"), "missing WebFetch in: {cli}");
        // Read should NOT appear — defaults are bypassed.
        assert!(!cli.contains("Read"), "unexpected Read from defaults in: {cli}");
    }

    #[test]
    fn resolve_capabilities_strict_mode_with_valid_caps_works() {
        // Strict mode only blocks empty; valid caps should still translate.
        let parsed = vec!["fs:write".to_string()];
        let tools = resolve_capabilities(&parsed, true, &[]).unwrap();
        assert!(tools.to_cli_string().contains("Write"));
    }

    // ── AllowedTools ───────────────────────────────────────────────────────────

    #[test]
    fn allowed_tools_to_cli_string_is_sorted() {
        // Determinism: same input always produces same string.
        let tools1 = translate_capabilities(&["fs:write".to_string()]).unwrap();
        let tools2 = translate_capabilities(&["fs:write".to_string()]).unwrap();
        assert_eq!(tools1.to_cli_string(), tools2.to_cli_string());
    }

    #[test]
    fn allowed_tools_empty_is_empty() {
        let tools = translate_capabilities(&[]).unwrap();
        assert!(tools.is_empty());
    }

    #[test]
    fn allowed_tools_non_empty_is_not_empty() {
        let tools = translate_capabilities(&["fs:read".to_string()]).unwrap();
        assert!(!tools.is_empty());
    }
}
