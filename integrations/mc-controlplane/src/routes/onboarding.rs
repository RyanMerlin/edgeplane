use axum::{extract::Query, http::HeaderMap, response::IntoResponse, routing::get, Json, Router};
use std::sync::Arc;

use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/agent-onboarding.json", get(agent_onboarding_manifest))
}

#[derive(serde::Deserialize)]
struct OnboardingQuery {
    endpoint: Option<String>,
}

async fn agent_onboarding_manifest(
    headers: HeaderMap,
    Query(q): Query<OnboardingQuery>,
) -> impl IntoResponse {
    let endpoint = q.endpoint.as_deref().unwrap_or("").trim().to_string();
    let base = normalize_endpoint(&endpoint, &headers);
    Json(build_manifest(&base)).into_response()
}

fn normalize_endpoint(endpoint: &str, headers: &HeaderMap) -> String {
    let raw = if endpoint.is_empty() {
        let host = headers
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("localhost");
        format!("https://{}", host)
    } else {
        endpoint.to_string()
    };

    // Ensure scheme
    let with_scheme = if raw.contains("://") {
        raw.clone()
    } else {
        format!("https://{}", raw)
    };

    // Strip to scheme://host only (no path)
    if let Some(after_scheme) = with_scheme.find("://").map(|i| i + 3) {
        let rest = &with_scheme[after_scheme..];
        let host_part = rest.split('/').next().unwrap_or(rest);
        let scheme = &with_scheme[..with_scheme.find("://").unwrap()];
        format!("{}://{}", scheme, host_part)
    } else {
        with_scheme
    }
}

fn build_manifest(base: &str) -> serde_json::Value {
    serde_json::json!({
        "name": "MissionControl Agent Onboarding",
        "version": "1.0",
        "integration_contract_version": "1.1.0",
        "generated_for_base_url": base,
        "endpoints": {
            "health": format!("{}/", base),
            "openapi": format!("{}/api/openapi.json", base),
            "explorer_tree": format!("{}/explorer/tree", base),
            "governance_active": format!("{}/governance/policy/active", base),
            "mcp_tools": format!("{}/mcp/tools", base),
            "mcp_call": format!("{}/mcp/call", base),
            "mcp_health": format!("{}/mcp/health", base),
            "skills_snapshot_resolve": format!("{}/skills/snapshots/resolve", base),
            "skills_sync_status": format!("{}/skills/sync/status", base),
            "ui": format!("{}/ui/", base)
        },
        "mcp_defaults": {
            "startup_timeout_sec": 45,
            "tool_timeout_sec": 60,
            "protocol_version": "2024-11-05",
            "healthcheck_path": "/",
            "endpoint_candidates": [base, "https://missioncontrol.internal.example", "http://localhost:8008"]
        },
        "mcp_server": {
            "name": "missioncontrol",
            "command": "mc",
            "args": ["serve"],
            "env": {"MC_BASE_URL": base, "MC_TOKEN": "${MC_TOKEN}"}
        },
        "mc_serve_mcp_server": {
            "name": "missioncontrol",
            "command": "mc",
            "args": ["serve"],
            "env": {"MC_BASE_URL": base}
        },
        "agent_configs": {
            "claude_code": {
                "missioncontrol": {"command": "mc", "args": ["serve"], "env": {"MC_BASE_URL": base}}
            },
            "codex": {
                "missioncontrol": {"command": "mc", "args": ["serve"], "env": {"MC_BASE_URL": base}}
            },
            "openclaw_custom": {
                "missioncontrol": {"command": "mc", "args": ["serve"], "env": {"MC_BASE_URL": base}}
            },
            "gemini": {
                "missioncontrol": {"command": "mc", "args": ["serve"], "env": {"MC_BASE_URL": base}}
            }
        },
        "bootstrap": {
            "remote_script": format!(
                "bash <(curl -fsSL https://raw.githubusercontent.com/missioncontrol-ai/mc-integration/main/install.sh) --endpoint {} --token ${{MC_TOKEN}} --agent both",
                base
            ),
            "local_script": format!(
                "bash install.sh --endpoint {} --token ${{MC_TOKEN}} --agent both",
                base
            )
        },
        "automation": {
            "config_generator_script": format!(
                "git clone https://github.com/missioncontrol-ai/mc-integration.git && cd mc-integration && bash install.sh --endpoint {} --token ${{MC_TOKEN}} --agent both",
                base
            )
        },
        "notes": [
            "Run `mc auth login` once to authenticate; mc serve reads the session token from disk.",
            "All agents now use `mc serve` (Rust-native MCP server) — no Python missioncontrol-mcp required.",
            "Set the activation endpoint to your MissionControl instance before copying configs.",
            "Public distribution repo: https://github.com/missioncontrol-ai/mc-integration",
            "Use missioncontrol-explorer for inline terminal tree views.",
            "`mc daemon` is optional and only needed for event streaming / Matrix integration."
        ]
    })
}
