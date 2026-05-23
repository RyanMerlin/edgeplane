use axum::{
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
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
                    "domains": "/domains",
                    "tasks": "/tasks",
                    "agents": "/agents",
                }
            })),
        )
            .into_response();
    }

    // Browser: redirect to the SvelteKit UI. API clients get JSON above.
    (
        StatusCode::FOUND,
        [(header::LOCATION, HeaderValue::from_static("/ui/"))],
    )
        .into_response()
}

async fn health_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
