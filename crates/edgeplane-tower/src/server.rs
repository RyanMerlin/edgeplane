use axum::{Router, extract::State, http::StatusCode, middleware, response::IntoResponse};
use sqlx::PgPool;
use std::{path::PathBuf, sync::Arc};
use tower_http::services::{ServeDir, ServeFile};

use crate::{auth, routes, state::{AppState, NodeInfo}};

#[derive(Default, Clone)]
pub struct AppConfig {
    pub node_id: u64,
    pub advertise_url: Option<String>,
    /// When set, routes not matched by this app are proxied to this base URL.
    pub api_proxy: Option<String>,
}

pub fn build_app(db: PgPool, config: AppConfig) -> Router {
    let state = Arc::new(AppState {
        db,
        node: NodeInfo {
            node_id: config.node_id,
            advertise_url: config.advertise_url.clone(),
            role: "standalone",
            term: 0,
            leader_id: None,
        },
        api_proxy: config.api_proxy.clone(),
    });

    // Phase 1.6: a single auth layer at the app boundary, applied only to
    // the controlplane's own routes. The layer consults
    // `auth::is_public_path` for the documented allowlist (health, OIDC
    // bootstrap, webhook receivers) and 401s everything else without a
    // valid credential. The proxy fallback sits OUTSIDE the layer so
    // requests for unknown paths (which the legacy backend handles with
    // its own auth) flow through unmolested.
    let authed = routes::build_router()
        .layer(middleware::from_fn_with_state(state.clone(), auth::require_auth));

    // Serve the SvelteKit web UI at /ui/ if EP_WEB_DIR points to the build.
    // Falls back to a 404 if the directory doesn't exist (e.g. in test builds).
    let web_dir = std::env::var("EP_WEB_DIR")
        .unwrap_or_else(|_| "/usr/local/share/edgeplane-web".to_string());
    let web_path = PathBuf::from(&web_dir);
    let router = if web_path.is_dir() {
        let serve = ServeDir::new(&web_path)
            .not_found_service(ServeFile::new(web_path.join("index.html")));
        Router::new()
            .merge(authed)
            .nest_service("/ui", serve)
            .fallback(proxy_fallback)
            .with_state(state)
    } else {
        Router::new()
            .merge(authed)
            .fallback(proxy_fallback)
            .with_state(state)
    };
    router
}

async fn proxy_fallback(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> axum::response::Response {
    let Some(base) = &state.api_proxy else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let path_query = req.uri().path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or_else(|| req.uri().path());
    let target = format!("{}{}", base.trim_end_matches('/'), path_query);

    let method_str = req.method().as_str().to_owned();
    let reqwest_method = reqwest::Method::from_bytes(method_str.as_bytes())
        .unwrap_or(reqwest::Method::GET);

    let body_bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    match reqwest::Client::new()
        .request(reqwest_method, &target)
        .body(body_bytes)
        .send()
        .await
    {
        Ok(r) => {
            let status = StatusCode::from_u16(r.status().as_u16())
                .unwrap_or(StatusCode::BAD_GATEWAY);
            match r.bytes().await {
                Ok(b) => (status, b).into_response(),
                Err(_) => StatusCode::BAD_GATEWAY.into_response(),
            }
        }
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}
