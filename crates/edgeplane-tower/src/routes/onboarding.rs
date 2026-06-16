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
        "name": "Edgeplane Agent Onboarding",
        "version": "1.0",
        "integration_contract_version": "1.1.0",
        "generated_for_base_url": base,
        "endpoints": {
            "health": format!("{}/", base),
            "openapi": format!("{}/api/openapi.json", base),
            "explorer_tree": format!("{}/explorer/tree", base),
            "mcp_tools": format!("{}/mcp/tools", base),
            "mcp_call": format!("{}/mcp/call", base),
            "mcp_health": format!("{}/mcp/health", base),
            "ui": format!("{}/ui/", base)
        },
        "mcp_defaults": {
            "startup_timeout_sec": 45,
            "tool_timeout_sec": 60,
            "protocol_version": "2024-11-05",
            "healthcheck_path": "/",
            "endpoint_candidates": [base, "https://edgeplane.internal.example", "http://localhost:8008"]
        },
        "mcp_server": {
            "name": "edgeplane",
            "command": "edgeplane",
            "args": ["serve"],
            "env": {"EP_BASE_URL": base}
        },
        "ep_serve_mcp_server": {
            "name": "edgeplane",
            "command": "edgeplane",
            "args": ["serve"],
            "env": {"EP_BASE_URL": base}
        },
        "agent_configs": {
            "claude_code": {
                "edgeplane": {"command": "edgeplane", "args": ["serve"], "env": {"EP_BASE_URL": base}}
            },
            "codex": {
                "edgeplane": {"command": "edgeplane", "args": ["serve"], "env": {"EP_BASE_URL": base}}
            },
            "openclaw_custom": {
                "edgeplane": {"command": "edgeplane", "args": ["serve"], "env": {"EP_BASE_URL": base}}
            },
            "gemini": {
                "edgeplane": {"command": "edgeplane", "args": ["serve"], "env": {"EP_BASE_URL": base}}
            }
        },
        "bootstrap": {
            "step_1": "edgeplane agent node join-token create --ttl-seconds 600",
            "step_2": format!(
                "edgeplaned register --join-token <TOKEN> --endpoint {}",
                base
            ),
            "remote_script": format!(
                "bash <(curl -fsSL https://raw.githubusercontent.com/RyanMerlin/edgeplane-integration/main/install.sh) --endpoint {} --join-token <TOKEN>",
                base
            ),
            "local_script": format!(
                "bash install.sh --endpoint {} --join-token <TOKEN>",
                base
            )
        },
        "automation": {
            "config_generator_script": format!(
                "git clone https://github.com/RyanMerlin/edgeplane-integration.git && cd edgeplane-integration && bash install.sh --endpoint {} --join-token <TOKEN>",
                base
            )
        },
        "notes": [
            "Run `edgeplane auth login` once to authenticate; edgeplane serve reads the session token from disk.",
            "All agents now use `edgeplane serve` (Rust-native MCP server) — no Python edgeplane-mcp required.",
            "Set the activation endpoint to your Edgeplane instance before copying configs.",
            "Public distribution repo: https://github.com/RyanMerlin/edgeplane-integration",
            "Use edgeplane-explorer for inline terminal tree views.",
            "`edgeplane daemon` is optional and only needed for event streaming / Matrix integration."
        ]
    })
}
