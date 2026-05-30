//! CLI schema discovery — emit the full CLI surface as a versioned JSON contract.
//!
//! Spec: docs/superpowers/specs/2026-05-25-cli-schema-federated-discovery-design.md
//!
//! The schema is intentionally self-contained: no shared crate with aria-rs.
//! The node structs are duplicated by design so both tools can evolve independently.

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
// Clap args struct (empty — no flags needed for this command)
// ---------------------------------------------------------------------------

#[derive(Args, Debug, Default)]
pub struct CliSchemaArgs {}

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
// Entry point
// ---------------------------------------------------------------------------

pub async fn run(_args: CliSchemaArgs) -> Result<()> {
    // Build the command tree from EdgeplaneCommand without pulling in the top-level
    // CliOpts from main.rs (which is not part of the library).
    let mut root_cmd = clap::Command::new("edgeplane")
        .about("Rust-native MCP bridge for Edgeplane")
        .version(env!("CARGO_PKG_VERSION"));
    root_cmd = crate::commands::EdgeplaneCommand::augment_subcommands(root_cmd);

    let root_node = build_node(&root_cmd, usize::MAX);

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
            .about("Rust-native MCP bridge for Edgeplane")
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
        let root_cmd = make_root();
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
        let root_cmd = make_root();
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
}
