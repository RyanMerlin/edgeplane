use axum::{
    extract::State,
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

/// Kubernetes probe endpoints, served at the *root* path (outside the `/api`
/// nest and outside the auth middleware) so kubelet — which cannot present a
/// credential — can reach them. The Helm chart and raw manifests probe
/// `/healthz` (liveness) and `/readyz` (readiness); before this router existed
/// both paths fell through to the proxy fallback and returned 404, so a
/// chart-based deploy could never reach Ready.
pub fn probe_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/healthz", get(healthz_handler))
        .route("/readyz", get(readyz_handler))
}

/// Liveness: the process is up and the async runtime is servicing requests.
/// Deliberately does no I/O — a slow or unavailable database must not restart
/// the pod (that is readiness's job).
async fn healthz_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

/// Readiness: the pod can serve traffic. Gated on a live database round-trip so
/// the pod is pulled from the Service endpoints when Postgres is unreachable,
/// instead of accepting requests that will 500.
async fn readyz_handler(State(state): State<Arc<AppState>>) -> Response {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "status": "ready" }))).into_response(),
        Err(e) => {
            tracing::warn!("readyz: database check failed: {e}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "not_ready", "reason": "database unavailable" })),
            )
                .into_response()
        }
    }
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
                "name": "Edgeplane API",
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
