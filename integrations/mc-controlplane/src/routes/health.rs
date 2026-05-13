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

    let auth_section = if has_session {
        r#"<section>
    <h2>Signed in</h2>
    <div class="links">
      <a href="/auth/logout">Sign out</a>
    </div>
  </section>"#
    } else {
        r#"<section>
    <h2>Access</h2>
    <div class="links">
      <a href="/auth/oidc/start">Sign in with SSO</a>
    </div>
  </section>"#
    };

    let html = Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>MissionControl</title>
  <style>
    body {{ font-family: system-ui, sans-serif; max-width: 600px; margin: 80px auto; padding: 0 24px; color: #1a1a1a; }}
    h1 {{ font-size: 1.8rem; margin-bottom: 4px; }}
    .version {{ color: #666; font-size: 0.9rem; margin-bottom: 32px; }}
    section {{ margin-bottom: 28px; }}
    h2 {{ font-size: 1rem; color: #444; margin-bottom: 10px; text-transform: uppercase; letter-spacing: 0.05em; }}
    .links {{ display: flex; flex-direction: column; gap: 10px; }}
    a {{ color: #0066cc; text-decoration: none; font-size: 1rem; }}
    a:hover {{ text-decoration: underline; }}
    .hint {{ color: #888; font-size: 0.85rem; margin-top: 40px; }}
    code {{ background: #f0f0f0; padding: 2px 6px; border-radius: 4px; font-size: 0.85rem; }}
  </style>
</head>
<body>
  <h1>MissionControl</h1>
  <div class="version">v{version} &mdash; API server online</div>
  {auth_section}
  <section>
    <h2>API</h2>
    <div class="links">
      <a href="/missions">Missions</a>
      <a href="/agents">Agents</a>
      <a href="/tasks">Tasks</a>
      <a href="/mcp/tools">MCP Tools</a>
    </div>
  </section>
  <p class="hint">CLI: <code>curl -fsSL https://raw.githubusercontent.com/RyanMerlin/missioncontrol/main/scripts/bootstrap-mc.sh | bash</code></p>
</body>
</html>"#
    ));

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
