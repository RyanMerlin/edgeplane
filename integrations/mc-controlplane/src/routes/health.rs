use axum::{
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
}

async fn root_handler(headers: HeaderMap) -> Response {
    let wants_json = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("application/json") && !v.contains("text/html"))
        .unwrap_or(false);

    if wants_json {
        return (
            StatusCode::OK,
            Json(json!({
                "name": "MissionControl API",
                "version": env!("CARGO_PKG_VERSION"),
                "status": "ok",
                "endpoints": {
                    "health": "/health",
                    "auth": "/auth/oidc/start",
                    "missions": "/missions",
                    "tasks": "/tasks",
                    "agents": "/agents",
                }
            })),
        )
            .into_response();
    }

    let has_session = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .map(|c| c.contains("mc_session_token="))
        .unwrap_or(false);

    let version = env!("CARGO_PKG_VERSION");

    let html = if has_session {
        Html(format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>MissionControl</title>
  <style>
    body {{ font-family: system-ui, sans-serif; max-width: 640px; margin: 80px auto; padding: 0 24px; color: #1a1a1a; }}
    h1 {{ font-size: 1.8rem; margin-bottom: 4px; }}
    .version {{ color: #666; font-size: 0.9rem; margin-bottom: 32px; }}
    .badge {{ display: inline-block; background: #dcfce7; color: #166534; border-radius: 6px; padding: 3px 10px; font-size: 0.85rem; font-weight: 600; margin-bottom: 28px; }}
    section {{ margin-bottom: 28px; }}
    h2 {{ font-size: 0.85rem; color: #444; margin-bottom: 10px; text-transform: uppercase; letter-spacing: 0.05em; }}
    .grid {{ display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }}
    .card {{ border: 1px solid #e2e2e2; border-radius: 8px; padding: 14px 16px; text-decoration: none; color: inherit; display: block; }}
    .card:hover {{ border-color: #0066cc; background: #f5f9ff; }}
    .card-title {{ font-weight: 600; margin-bottom: 3px; }}
    .card-desc {{ font-size: 0.82rem; color: #666; }}
    .signout {{ color: #666; font-size: 0.85rem; }}
    .signout a {{ color: #cc0000; text-decoration: none; }}
    .signout a:hover {{ text-decoration: underline; }}
    .hint {{ color: #888; font-size: 0.82rem; margin-top: 32px; border-top: 1px solid #eee; padding-top: 16px; }}
    code {{ background: #f0f0f0; padding: 2px 6px; border-radius: 4px; font-size: 0.82rem; }}
  </style>
</head>
<body>
  <h1>MissionControl</h1>
  <div class="version">v{version}</div>
  <div class="badge">&#10003; Signed in</div>
  <p style="margin-bottom:24px"><a class="signin-btn" href="/ui/" style="background:#0066cc;color:white;padding:10px 20px;border-radius:6px;text-decoration:none;font-weight:500">Open Dashboard</a></p>
  <section>
    <h2>API</h2>
    <div class="grid">
      <a class="card" href="/missions">
        <div class="card-title">Missions</div>
        <div class="card-desc">Active mission contexts</div>
      </a>
      <a class="card" href="/agents">
        <div class="card-title">Agents</div>
        <div class="card-desc">Registered fleet agents</div>
      </a>
      <a class="card" href="/tasks">
        <div class="card-title">Tasks</div>
        <div class="card-desc">Task queue and assignments</div>
      </a>
      <a class="card" href="/mcp/tools">
        <div class="card-title">MCP Tools</div>
        <div class="card-desc">Available tool manifest</div>
      </a>
    </div>
  </section>
  <p class="signout"><a href="/auth/logout">Sign out</a></p>
  <p class="hint">Full interface: <code>mc tui</code> &nbsp;&bull;&nbsp; Install: <code>curl -fsSL https://raw.githubusercontent.com/RyanMerlin/missioncontrol/main/scripts/bootstrap-mc.sh | bash</code></p>
</body>
</html>"#))
    } else {
        Html(format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>MissionControl</title>
  <style>
    body {{ font-family: system-ui, sans-serif; max-width: 600px; margin: 80px auto; padding: 0 24px; color: #1a1a1a; }}
    h1 {{ font-size: 1.8rem; margin-bottom: 4px; }}
    .version {{ color: #666; font-size: 0.9rem; margin-bottom: 32px; }}
    .tagline {{ color: #444; margin-bottom: 28px; }}
    .signin-btn {{
      display: inline-block; background: #0066cc; color: white; padding: 10px 24px;
      border-radius: 6px; text-decoration: none; font-size: 1rem; font-weight: 500;
    }}
    .signin-btn:hover {{ background: #0052a3; }}
    .hint {{ color: #888; font-size: 0.85rem; margin-top: 40px; }}
    code {{ background: #f0f0f0; padding: 2px 6px; border-radius: 4px; font-size: 0.85rem; }}
  </style>
</head>
<body>
  <h1>MissionControl</h1>
  <div class="version">v{version} &mdash; API server online</div>
  <p class="tagline">AI agent fleet orchestration platform.</p>
  <a class="signin-btn" href="/auth/oidc/start">Sign in with SSO</a>
  <p class="hint">CLI: <code>curl -fsSL https://raw.githubusercontent.com/RyanMerlin/missioncontrol/main/scripts/bootstrap-mc.sh | bash</code></p>
</body>
</html>"#))
    };

    // Prevent CDN/proxy caching — this page varies by cookie.
    let mut resp = html.into_response();
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    resp
}

async fn health_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
