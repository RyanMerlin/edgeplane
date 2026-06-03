//! Regression tests for web asset Cache-Control headers.
//!
//! Root cause these guard against: `ServeDir` set no Cache-Control, so browsers
//! heuristically cached the HTML entrypoint. After a redeploy the content-hashed
//! asset filenames rotate, and a stale cached `index.html` then 404s every
//! asset → unstyled, non-interactive page ("white page, dead OIDC button").
//!
//! Contract:
//!   - HTML entrypoint   → `no-cache` (always revalidate; deploys self-heal)
//!   - `/assets/...` (Vite) or `/_app/...` asset → `public, max-age=31536000, immutable`

use axum_test::TestServer;
use edgeplane_tower::{AppConfig, build_app};
use sqlx::PgPool;
use std::fs;
use std::path::PathBuf;

fn test_pool() -> PgPool {
    PgPool::connect_lazy("postgres://localhost/test").expect("lazy pool")
}

/// Build a throwaway web dir with an index.html and a hashed asset, point
/// EP_WEB_DIR at it, and return the configured app.
fn app_with_web_dir() -> TestServer {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("ep_web_cache_test_{}", std::process::id()));
    let assets = dir.join("assets");
    fs::create_dir_all(&assets).expect("mkdir web dir");
    fs::write(
        dir.join("index.html"),
        "<!doctype html><html><body>ok</body></html>",
    )
    .expect("write index.html");
    fs::write(assets.join("index-DEADBEEF.js"), "export const x = 1;").expect("write asset");

    // SAFETY: single-threaded setup before the server is built; this test
    // binary owns the process environment.
    unsafe {
        std::env::set_var("EP_WEB_DIR", &dir);
    }

    TestServer::new(build_app(test_pool(), AppConfig::default()))
}

#[tokio::test]
async fn html_entrypoint_is_no_cache() {
    let server = app_with_web_dir();
    let res = server.get("/").await;
    res.assert_status_ok();
    let cc = res
        .headers()
        .get("cache-control")
        .expect("Cache-Control present on HTML")
        .to_str()
        .unwrap();
    assert_eq!(cc, "no-cache", "HTML entrypoint must always revalidate");
}

#[tokio::test]
async fn hashed_asset_is_immutable() {
    let server = app_with_web_dir();
    let res = server.get("/assets/index-DEADBEEF.js").await;
    res.assert_status_ok();
    let cc = res
        .headers()
        .get("cache-control")
        .expect("Cache-Control present on asset")
        .to_str()
        .unwrap();
    assert_eq!(cc, "public, max-age=31536000, immutable");
}
