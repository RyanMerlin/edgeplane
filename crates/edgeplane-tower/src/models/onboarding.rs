//! Typed DTOs for the onboarding manifest endpoint.
//!
//! `GET /api/agent-onboarding.json` returns a static `serde_json::Value`
//! built by `routes/onboarding.rs::build_manifest`. The struct here is a
//! **mirror DTO** matching the wire shape exactly, used only to generate a
//! typed OpenAPI schema for the frontend client.
//!
//! MIRROR: handler still returns `Json(build_manifest(...))` which is a
//! `serde_json::Value`. Converting the handler would be straightforward but
//! is deferred because the manifest is display-only and the shape is stable.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

/// Endpoint URLs embedded in the onboarding manifest.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OnboardingEndpoints {
    pub health: String,
    pub openapi: String,
    pub explorer_tree: String,
    pub governance_active: String,
    pub mcp_tools: String,
    pub mcp_call: String,
    pub mcp_health: String,
    pub ui: String,
}

/// MCP server config block embedded in the onboarding manifest.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// Environment variables required to launch the MCP server.
    pub env: HashMap<String, String>,
}

/// MCP defaults block.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct McpDefaults {
    pub startup_timeout_sec: u32,
    pub tool_timeout_sec: u32,
    pub protocol_version: String,
    pub healthcheck_path: String,
    pub endpoint_candidates: Vec<String>,
}

/// Bootstrap instructions embedded in the onboarding manifest.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OnboardingBootstrap {
    pub step_1: String,
    pub step_2: String,
    pub remote_script: String,
    pub local_script: String,
}

/// Automation block.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OnboardingAutomation {
    pub config_generator_script: String,
}

/// The onboarding manifest returned by `GET /api/agent-onboarding.json`.
///
/// Describes how to connect an agent runtime (edgeplaned) to this EdgePlane
/// instance. No auth required — the manifest itself contains no secrets.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OnboardingManifest {
    pub name: String,
    pub version: String,
    pub integration_contract_version: String,
    pub generated_for_base_url: String,
    pub endpoints: OnboardingEndpoints,
    pub mcp_defaults: McpDefaults,
    pub mcp_server: McpServerConfig,
    pub ep_serve_mcp_server: McpServerConfig,
    /// Per-agent-runtime config snippets (keys: `"claude_code"`, `"codex"`, etc.).
    pub agent_configs: serde_json::Value,
    pub bootstrap: OnboardingBootstrap,
    pub automation: OnboardingAutomation,
    pub notes: Vec<String>,
}
