use axum::{
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
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

async fn root_handler(headers: HeaderMap) -> impl IntoResponse {
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

    let version = env!("CARGO_PKG_VERSION");
    Html(format!(
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
    .links {{ display: flex; flex-direction: column; gap: 12px; }}
    a {{ color: #0066cc; text-decoration: none; font-size: 1rem; }}
    a:hover {{ text-decoration: underline; }}
    .hint {{ color: #888; font-size: 0.85rem; margin-top: 40px; }}
    code {{ background: #f0f0f0; padding: 2px 6px; border-radius: 4px; font-size: 0.9rem; }}
  </style>
</head>
<body>
  <h1>MissionControl</h1>
  <div class="version">v{version} &mdash; API server online</div>
  <div class="links">
    <a href="/auth/oidc/start">Sign in</a>
    <a href="/health">Health check</a>
  </div>
  <p class="hint">Install the CLI: <code>curl -fsSL https://raw.githubusercontent.com/RyanMerlin/missioncontrol/main/scripts/bootstrap-mc.sh | bash</code></p>
</body>
</html>"#
    ))
    .into_response()
}

async fn health_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
