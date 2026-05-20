/// `mc capabilities` — list and describe built-in capability packs.
use anyhow::Result;
use clap::Subcommand;
use mcd_packs::PackRegistry;

// ---------------------------------------------------------------------------
// Clap types
// ---------------------------------------------------------------------------

#[derive(Subcommand, Debug)]
pub enum CapabilitiesCmd {
    /// List available capabilities.
    #[command(name = "list", alias = "ls", about = "List available capabilities")]
    List {
        /// Filter by tag (e.g. kubernetes, git).
        #[arg(long)]
        tag: Option<String>,
        /// Output as JSON.
        #[arg(long)]
        json: bool,
        /// Routing mode override (auto | local | backend | remote).
        #[arg(long, value_name = "MODE")]
        route: Option<String>,
    },
    /// Show full schema for a capability.
    #[command(about = "Show full schema for a capability")]
    Describe {
        /// The capability name in pack.capability format (e.g. kubectl-observe.kubectl-get-pods).
        name: String,
    },
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub async fn run(
    cmd: CapabilitiesCmd,
    host: Option<String>,
    route: Option<String>,
) -> Result<()> {
    match cmd {
        CapabilitiesCmd::List { tag, json, route: route_override } => {
            // The --route on the subcommand takes precedence over the top-level --route.
            let effective_route = route_override.or(route);
            run_list(tag.as_deref(), json, host, effective_route).await
        }
        CapabilitiesCmd::Describe { name } => run_describe(&name),
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

async fn run_list(
    tag: Option<&str>,
    as_json: bool,
    host: Option<String>,
    route_override: Option<String>,
) -> Result<()> {
    use crate::dispatch::McDispatch;
    use std::io::IsTerminal;

    // Try the daemon first.
    let dispatch = McDispatch::from_env(host, route_override);
    let daemon_result = dispatch.list_capabilities(tag).await;

    match daemon_result {
        Ok(items) => {
            if as_json || !std::io::stdout().is_terminal() {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                for item in &items {
                    let full_name = item["full_name"].as_str().unwrap_or("?");
                    let description = item["description"].as_str().unwrap_or("");
                    println!("{full_name} — {description}");
                }
            }
        }
        Err(_) => {
            // Daemon offline — fall back to embedded registry.
            eprintln!("⚠ daemon offline — showing all built-ins, policy not applied");

            let registry = PackRegistry::load_builtin()
                .map_err(|e| anyhow::anyhow!("failed to load built-in packs: {e}"))?;

            let summaries = registry.capabilities(tag);

            if as_json || !std::io::stdout().is_terminal() {
                let json_items: Vec<serde_json::Value> = summaries
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "full_name": s.full_name,
                            "description": s.description,
                            "tags": s.tags,
                            "risk": s.risk.to_string(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&json_items)?);
            } else {
                for s in &summaries {
                    println!("{} — {}", s.full_name, s.description);
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// describe
// ---------------------------------------------------------------------------

fn run_describe(name: &str) -> Result<()> {
    let registry = PackRegistry::load_builtin()
        .map_err(|e| anyhow::anyhow!("failed to load built-in packs: {e}"))?;

    match registry.get_by_full_name(name) {
        Some(cap) => {
            let json = serde_json::to_string_pretty(cap)
                .map_err(|e| anyhow::anyhow!("failed to serialize capability: {e}"))?;
            println!("{json}");
            Ok(())
        }
        None => anyhow::bail!("capability '{}' not found", name),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_known_capability_returns_json() {
        let registry = PackRegistry::load_builtin()
            .expect("built-in packs should load");

        // Pick the first capability from the registry as the known capability.
        let first = registry.capabilities(None).into_iter().next()
            .expect("built-in packs should have at least one capability");

        let cap = registry.get_by_full_name(&first.full_name);
        assert!(cap.is_some(), "get_by_full_name should find a known capability");

        let cap = cap.unwrap();
        // The capability name stored in the manifest is the short name (without pack prefix).
        // The full_name in CapabilitySummary is "<pack>.<cap_name>".
        let short_name = first.full_name
            .splitn(2, '.')
            .nth(1)
            .unwrap_or(&first.full_name);
        assert_eq!(cap.name, short_name, "manifest name should match the short capability name");

        let serialized = serde_json::to_string(cap).expect("should serialize");
        assert!(!serialized.is_empty());
        assert!(serialized.contains(&cap.name));
    }

    #[test]
    fn describe_unknown_capability_returns_none() {
        let registry = PackRegistry::load_builtin()
            .expect("built-in packs should load");
        let result = registry.get_by_full_name("nonexistent.nope");
        assert!(result.is_none(), "unknown capability should return None");
    }

    #[test]
    fn list_with_tag_filters_correctly() {
        let registry = PackRegistry::load_builtin()
            .expect("built-in packs should load");

        let kubernetes_caps = registry.capabilities(Some("kubernetes"));
        let all_caps = registry.capabilities(None);

        assert!(
            kubernetes_caps.len() < all_caps.len(),
            "tag filter should reduce the result set"
        );
        for cap in &kubernetes_caps {
            assert!(
                cap.tags.iter().any(|t| t == "kubernetes"),
                "capability {} should have kubernetes tag",
                cap.full_name
            );
        }
    }

    #[test]
    fn list_all_returns_nonempty() {
        let registry = PackRegistry::load_builtin()
            .expect("built-in packs should load");
        let all = registry.capabilities(None);
        assert!(!all.is_empty(), "built-in packs should have capabilities");
    }
}
