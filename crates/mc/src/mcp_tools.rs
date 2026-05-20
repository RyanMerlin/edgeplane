use crate::{booster::AgentBooster, client::MissionControlClient, schema_pack::SchemaPack};
use anyhow::{Context, Result};
use serde_json::{Value, json};

/// Fetch the raw tools list from the backend's /mcp/tools endpoint.
/// Shared by the MCP server cache and background warm-up.
pub async fn fetch_tools_from_backend(client: &MissionControlClient) -> Result<Vec<Value>> {
    let response = client
        .get_json("/mcp/tools")
        .await
        .context("failed to fetch tools from backend")?;

    let tools: Vec<Value> = match response {
        Value::Array(arr) => arr.into_iter().map(normalize_tool).collect(),
        Value::Object(ref obj) => obj
            .get("tools")
            .and_then(|t| t.as_array())
            .map(|arr| arr.iter().cloned().map(normalize_tool).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    Ok(tools)
}

fn normalize_tool(mut tool: Value) -> Value {
    let Some(obj) = tool.as_object_mut() else {
        return tool;
    };

    if !obj.contains_key("inputSchema") {
        let schema = obj
            .remove("input_schema")
            .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
        obj.insert("inputSchema".to_string(), schema);
    }

    tool
}

pub async fn call_tool(
    client: &MissionControlClient,
    booster: Option<&AgentBooster>,
    schema_pack: Option<&SchemaPack>,
    tool: &str,
    args: Value,
) -> Result<Value> {
    if let Some(schema_pack) = schema_pack {
        schema_pack
            .validate_payload(&args)
            .context("schema pack validation failed")?;
    }

    if let Some(booster) = booster {
        if booster.is_enabled() {
            let short_circuit = booster.run(&args).context("booster validation failed")?;
            if short_circuit {
                if booster.allow_short_circuit() {
                    println!("[booster] short-circuited {tool}");
                    return Ok(json!({ "booster_short_circuit": true, "tool": tool }));
                }
                println!(
                    "[booster] short-circuit requested for {tool} but disabled; forwarding to Mission Control"
                );
            }
        }
    }

    let request = json!({
        "tool": tool,
        "args": args,
    });
    client.post_json("/mcp/call", &request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_tool_renames_snake_case_schema() {
        let tool = json!({
            "name": "example",
            "description": "test",
            "input_schema": {
                "type": "object",
                "properties": {}
            }
        });

        let normalized = normalize_tool(tool);
        assert!(normalized.get("input_schema").is_none());
        assert!(normalized.get("inputSchema").is_some());
    }

    #[test]
    fn normalize_tool_inserts_empty_schema_when_missing() {
        let tool = json!({
            "name": "example",
            "description": "test"
        });

        let normalized = normalize_tool(tool);
        assert_eq!(
            normalized.get("inputSchema"),
            Some(&json!({"type": "object", "properties": {}}))
        );
    }
}
