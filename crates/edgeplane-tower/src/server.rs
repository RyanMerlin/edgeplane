use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use base64::Engine;
use sqlx::PgPool;
use std::{path::PathBuf, sync::Arc};
use tower_http::services::{ServeDir, ServeFile};

use crate::{auth, jwt, routes, state::{AppState, NodeInfo}};

#[derive(Default, Clone)]
pub struct AppConfig {
    pub node_id: u64,
    pub advertise_url: Option<String>,
    /// When set, routes not matched by this app are proxied to this base URL.
    pub api_proxy: Option<String>,
    /// Lowercased admin emails, parsed from `EP_ADMIN_EMAILS` at the entrypoint.
    pub admin_emails: std::collections::HashSet<String>,
    /// Admin group names (exact, case-sensitive), parsed from `EP_ADMIN_GROUPS`.
    pub admin_groups: std::collections::HashSet<String>,
}

pub fn build_app(db: PgPool, config: AppConfig) -> Router {
    // Load JWT signing key from EP_JWT_SIGNING_KEY (base64-encoded PKCS#8 PEM).
    // If unset, auto-generate an ephemeral keypair and warn — dev mode only.
    let (jwt_encoding_key, jwt_decoding_key) = load_jwt_keys();

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
        jwt_encoding_key,
        jwt_decoding_key,
        admin_emails: config.admin_emails.clone(),
        admin_groups: config.admin_groups.clone(),
    });

    // Phase 2: API routes are nested under /api. The auth middleware is
    // layered on the routes before nesting, so handlers still see paths
    // without the /api prefix (e.g. `/agents`, `/health`). The
    // `auth::is_public_path` allowlist therefore does not need updating.
    let authed = routes::build_router()
        .layer(middleware::from_fn_with_state(state.clone(), auth::require_auth));

    // Serve the React/Vite web SPA at root (/) if EP_WEB_DIR points to the
    // build. SPA fallback via fallback_service handles client-side routing.
    // Falls back to proxy_fallback when the web build is absent (test builds,
    // dev without a web build). `web_cache_control` applies the
    // immutable-asset / no-cache-HTML split (see fn docs).
    let web_dir = std::env::var("EP_WEB_DIR")
        .unwrap_or_else(|_| "/usr/local/share/edgeplane-web".to_string());
    let web_path = PathBuf::from(&web_dir);
    if web_path.is_dir() {
        let serve = ServeDir::new(&web_path)
            .not_found_service(ServeFile::new(web_path.join("index.html")));
        Router::new()
            .nest("/api", authed)
            .fallback_service(serve)
            .layer(middleware::from_fn(web_cache_control))
            .with_state(state)
    } else {
        Router::new()
            .nest("/api", authed)
            .fallback(proxy_fallback)
            .with_state(state)
    }
}

/// Apply Cache-Control to statically-served web responses.
///
/// Vite (and SvelteKit before it) emit content-hashed asset filenames under
/// `/assets/` and `/_app/` (e.g. `/assets/index-CmfekmjZ.js`). The filename
/// *is* the cache key, so those are safe to cache forever (`immutable`). The
/// HTML entrypoint (`index.html`, and the SPA fallback) must NOT be cached
/// without revalidation: it references the *current* build's hashed filenames,
/// so a browser-cached stale `index.html` will request asset hashes that no
/// longer exist after a redeploy → every CSS/JS request 404s → the page renders
/// unstyled and never hydrates (dead buttons). `no-cache` forces the browser to
/// revalidate the document on every load (cheap 304 when unchanged), which
/// makes deploys self-healing instead of breaking ~half the time.
///
/// Without this, `tower_http::ServeDir` sets no Cache-Control at all and
/// browsers fall back to heuristic caching of the HTML — the root cause of the
/// intermittent "white page / dead OIDC button after deploy" failures.
async fn web_cache_control(req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    let mut resp = next.run(req).await;

    let hashed = path.starts_with("/assets/") || path.starts_with("/_app/");
    if hashed {
        resp.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    } else if resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/html"))
    {
        resp.headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }

    resp
}

fn load_jwt_keys() -> (jsonwebtoken::EncodingKey, jsonwebtoken::DecodingKey) {
    use rsa::{RsaPrivateKey, pkcs8::{DecodePrivateKey, EncodePublicKey, LineEnding}};

    if let Ok(b64) = std::env::var("EP_JWT_SIGNING_KEY") {
        let pem_bytes = base64::engine::general_purpose::STANDARD
            .decode(&b64)
            .expect("EP_JWT_SIGNING_KEY must be base64-encoded");
        let pem = String::from_utf8(pem_bytes)
            .expect("EP_JWT_SIGNING_KEY decoded value is not valid UTF-8");
        let enc = jwt::encoding_key_from_pem(&pem)
            .expect("EP_JWT_SIGNING_KEY: invalid RSA PKCS#8 PEM");
        let pub_pem = RsaPrivateKey::from_pkcs8_pem(&pem)
            .expect("EP_JWT_SIGNING_KEY: cannot parse PKCS#8")
            .to_public_key()
            .to_public_key_pem(LineEnding::LF)
            .expect("public key PEM export failed");
        let dec = jwt::decoding_key_from_pem(&pub_pem)
            .expect("EP_JWT_SIGNING_KEY: public key error");
        (enc, dec)
    } else {
        tracing::warn!(
            "EP_JWT_SIGNING_KEY not set — generating ephemeral RSA keypair. \
             Node JWTs will be invalid after restart. Set EP_JWT_SIGNING_KEY for production."
        );
        let (priv_pem, pub_pem) = jwt::generate_rsa_keypair().expect("RSA keygen failed");
        let enc = jwt::encoding_key_from_pem(&priv_pem).unwrap();
        let dec = jwt::decoding_key_from_pem(&pub_pem).unwrap();
        (enc, dec)
    }
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

    // Capture the Authorization header before consuming the body.
    let auth_header = req.headers().get(axum::http::header::AUTHORIZATION).cloned();

    let body_bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    let mut proxy_request = reqwest::Client::new()
        .request(reqwest_method, &target)
        .body(body_bytes);

    // Forward the Authorization header so the backend can make its own auth decision.
    if let Some(auth_value) = auth_header {
        proxy_request = proxy_request.header(reqwest::header::AUTHORIZATION, auth_value.as_bytes());
    }

    match proxy_request
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
