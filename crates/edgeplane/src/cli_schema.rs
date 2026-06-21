//! CLI schema discovery — emit the full CLI surface as a versioned JSON contract.
//!
//! The schema is intentionally self-contained: no shared crate with external tooling.
//! The node structs are duplicated by design so both tools can evolve independently.
//!
//! Exposed as `edgeplane discover [path...] [--deep]`.

use anyhow::Result;
use clap::{Args, Subcommand};
use serde::Serialize;

// ---------------------------------------------------------------------------
// Schema node types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ArgNode {
    pub name: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

#[derive(Serialize)]
pub struct OptionNode {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short: Option<char>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    pub takes_value: bool,
}

#[derive(Serialize)]
pub struct CapabilityNode {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<ArgNode>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<OptionNode>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subcommands: Vec<CapabilityNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
}

#[derive(Serialize)]
pub struct CliSchema {
    pub schema_version: u32,
    pub binary: String,
    pub version: String,
    pub root: CapabilityNode,
}

// ---------------------------------------------------------------------------
// Clap args struct
// ---------------------------------------------------------------------------

#[derive(Args, Debug, Default)]
pub struct DiscoverArgs {
    /// Drill into a specific subcommand path (e.g. `agent`, `agent signal`).
    pub path: Vec<String>,
    /// Return the full subtree (default: 1 level of subcommands).
    #[arg(long)]
    pub deep: bool,
}

// ---------------------------------------------------------------------------
// Tree walker
// ---------------------------------------------------------------------------

/// Walk a clap `Command` tree up to `remaining_depth` levels deep.
///
/// `remaining_depth = 0` means: capture this node's args/options but don't
/// recurse into subcommands.  Pass `usize::MAX` for unlimited depth.
pub fn build_node(cmd: &clap::Command, remaining_depth: usize) -> CapabilityNode {
    let name = cmd.get_name().to_string();
    let about = cmd.get_about().map(|s| s.to_string()).filter(|s| !s.is_empty());
    let aliases: Vec<String> = cmd
        .get_all_aliases()
        .map(|a| a.to_string())
        .collect();

    let hidden = if cmd.is_hide_set() { Some(true) } else { None };

    let mut args = Vec::new();
    let mut options = Vec::new();

    for arg in cmd.get_arguments() {
        if arg.is_hide_set() {
            continue;
        }
        // Skip the built-in help flag — it's noise in a schema contract.
        if arg.get_id() == "help" {
            continue;
        }

        if arg.is_positional() {
            args.push(ArgNode {
                name: arg.get_id().to_string(),
                required: arg.is_required_set(),
                help: arg.get_help().map(|s| s.to_string()),
            });
        } else {
            let name = arg
                .get_long()
                .map(|s| s.to_string())
                .unwrap_or_else(|| arg.get_id().to_string());
            let takes_value = arg
                .get_num_args()
                .map(|n| n.takes_values())
                .unwrap_or(false);
            let default = arg
                .get_default_values()
                .first()
                .and_then(|v| v.to_str())
                .map(|s| s.to_string());
            options.push(OptionNode {
                name,
                short: arg.get_short(),
                help: arg.get_help().map(|s| s.to_string()),
                default,
                takes_value,
            });
        }
    }

    let subcommands = if remaining_depth == 0 {
        Vec::new()
    } else {
        cmd.get_subcommands()
            .filter(|sub| !sub.is_hide_set())
            .map(|sub| build_node(sub, remaining_depth.saturating_sub(1)))
            .collect()
    };

    CapabilityNode {
        name,
        about,
        aliases,
        args,
        options,
        subcommands,
        hidden,
    }
}

// ---------------------------------------------------------------------------
// Meta-tool helper (used by the MCP gateway's discover meta-tool)
// ---------------------------------------------------------------------------

/// Walk the edgeplane command tree by `path` tokens and return the subtree at
/// that node as a JSON value.  `deep=false` returns 1 level; `deep=true` returns
/// the full subtree.  Used by the `discover` MCP meta-tool so the gateway can
/// serve it without spawning a subprocess.
pub fn discover_to_value(path: &[String], deep: bool) -> serde_json::Value {
    use crate::commands::CliRoot;
    use clap::CommandFactory;

    let mut root_cmd = <CliRoot as CommandFactory>::command();
    root_cmd.build();

    let depth = if deep { usize::MAX } else { 1 };

    // Walk path tokens. On unknown token, return an error object.
    let mut current: &clap::Command = &root_cmd;
    for token in path {
        match current
            .get_subcommands()
            .find(|s| s.get_name() == token.as_str())
        {
            Some(sub) => current = sub,
            None => {
                return serde_json::json!({
                    "error": format!("unknown path segment '{}' — run discover() for top-level", token)
                });
            }
        }
    }

    serde_json::to_value(build_node(current, depth)).unwrap_or_else(|e| {
        serde_json::json!({ "error": format!("serialization failed: {}", e) })
    })
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub async fn run(args: DiscoverArgs) -> Result<()> {
    // Build the command tree from EdgeplaneCommand without pulling in the top-level
    // CliOpts from main.rs (which is not part of the library).
    let mut root_cmd = clap::Command::new("edgeplane")
        .about("EdgePlane — fleet control-plane CLI")
        .version(env!("CARGO_PKG_VERSION"));
    root_cmd = crate::commands::EdgeplaneCommand::augment_subcommands(root_cmd);
    // build() resolves all subcommand names (including `name = "..."` overrides)
    // before we walk the tree.
    root_cmd.build();

    // Walk the tree by path tokens to find the subtree to emit.
    let mut cursor: &clap::Command = &root_cmd;
    for token in &args.path {
        match cursor
            .get_subcommands()
            .find(|s| s.get_name() == token.as_str())
        {
            Some(sub) => cursor = sub,
            None => anyhow::bail!(
                "unknown subcommand path segment '{}' — run `edgeplane discover` for top-level",
                token
            ),
        }
    }

    let depth = if args.deep { usize::MAX } else { 1 };
    let root_node = build_node(cursor, depth);

    let schema = CliSchema {
        schema_version: 1,
        binary: "edgeplane".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        root: root_node,
    };

    println!("{}", serde_json::to_string(&schema)?);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_root() -> clap::Command {
        let mut root = clap::Command::new("edgeplane")
            .about("EdgePlane — fleet control-plane CLI")
            .version(env!("CARGO_PKG_VERSION"));
        root = crate::commands::EdgeplaneCommand::augment_subcommands(root);
        root
    }

    #[test]
    fn schema_envelope_is_correct() {
        let root_cmd = make_root();
        let root_node = build_node(&root_cmd, usize::MAX);
        let schema = CliSchema {
            schema_version: 1,
            binary: "edgeplane".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            root: root_node,
        };

        assert_eq!(schema.schema_version, 1);
        assert_eq!(schema.binary, "edgeplane");
        assert!(
            !schema.root.subcommands.is_empty(),
            "root should have subcommands"
        );
    }

    #[test]
    fn build_node_depth_one_has_no_nested_subcommands() {
        let mut root_cmd = make_root();
        root_cmd.build();
        // At depth=1 the root node's direct children are captured but their
        // children are dropped (remaining_depth goes to 0 on the recursive call).
        let node = build_node(&root_cmd, 1);

        // Every direct child should have empty subcommands.
        for child in &node.subcommands {
            assert!(
                child.subcommands.is_empty(),
                "child '{}' should have no nested subcommands at depth=1",
                child.name
            );
        }
    }

    #[test]
    fn build_node_full_depth_expands_nested() {
        let mut root_cmd = make_root();
        root_cmd.build();
        let node = build_node(&root_cmd, usize::MAX);

        // The `agent` subcommand has nested subcommands (signal, cancel, list, …).
        let agent = node
            .subcommands
            .iter()
            .find(|s| s.name == "agent")
            .expect("agent subcommand should exist");

        assert!(
            !agent.subcommands.is_empty(),
            "agent should have nested subcommands at full depth"
        );
    }

    #[test]
    fn discover_to_value_top_level_returns_subcommands() {
        let v = discover_to_value(&[], false);
        assert!(v.get("subcommands").is_some(), "top-level should have subcommands");
        let subs = v.get("subcommands").unwrap().as_array().unwrap();
        assert!(!subs.is_empty());
    }

    #[test]
    fn discover_to_value_path_drills_into_subtree() {
        let v = discover_to_value(&["domain".to_string()], false);
        let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("");
        assert_eq!(name, "domain");
    }

    #[test]
    fn discover_to_value_unknown_path_returns_error() {
        let v = discover_to_value(&["no-such-command".to_string()], false);
        assert!(v.get("error").is_some(), "unknown path should return error object");
    }
}
