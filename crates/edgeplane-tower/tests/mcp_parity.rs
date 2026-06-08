//! Catalogue↔dispatch parity test.
//!
//! Asserts that every tool advertised in `list_tools()` has a corresponding
//! dispatch arm, and every dispatch arm corresponds to an advertised tool.
//! Prevents the catalogue from silently drifting — e.g. adding a tool to
//! `list_tools()` without a dispatch arm (or vice-versa).
//!
//! Additionally verifies that the HTTP catalogue (`GET /api/mcp/tools`)
//! returns exactly the 24 expected tools.

use axum_test::TestServer;
use edgeplane_tower::{build_app, routes::mcp, AppConfig};
use sqlx::PgPool;

fn test_pool() -> PgPool {
    PgPool::connect_lazy("postgres://localhost/test").expect("lazy pool")
}

fn server() -> TestServer {
    TestServer::new(build_app(test_pool(), AppConfig::default()))
}

/// Every name in `advertised_tool_names()` must appear in `dispatch_handled_names()`.
#[test]
fn advertised_tools_all_have_dispatch_arm() {
    let advertised: std::collections::HashSet<_> = mcp::advertised_tool_names().into_iter().collect();
    let handled: std::collections::HashSet<_> = mcp::dispatch_handled_names().into_iter().collect();

    let missing_dispatch: Vec<_> = advertised.difference(&handled).copied().collect();
    assert!(
        missing_dispatch.is_empty(),
        "tools advertised in list_tools() but missing a dispatch arm: {missing_dispatch:?}\n\
         Add matching arms to dispatch() or remove from list_tools()."
    );
}

/// Every name in `dispatch_handled_names()` must appear in `advertised_tool_names()`.
#[test]
fn dispatch_arms_all_have_catalogue_entry() {
    let advertised: std::collections::HashSet<_> = mcp::advertised_tool_names().into_iter().collect();
    let handled: std::collections::HashSet<_> = mcp::dispatch_handled_names().into_iter().collect();

    let extra_dispatch: Vec<_> = handled.difference(&advertised).copied().collect();
    assert!(
        extra_dispatch.is_empty(),
        "tools with dispatch arms but not in list_tools(): {extra_dispatch:?}\n\
         Add matching entries to list_tools() or remove the dispatch arm."
    );
}

/// The HTTP catalogue must return exactly 24 tools (ADR 0006 runtime set).
#[tokio::test]
async fn http_catalogue_has_exactly_24_tools() {
    let res = server().get("/api/mcp/tools").await;
    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    let tools = body.as_array().expect("list_tools must return an array");
    assert_eq!(
        tools.len(),
        24,
        "expected exactly 24 tools in the MCP catalogue (ADR 0006), found {}",
        tools.len()
    );
}

/// The HTTP catalogue must contain all names from `advertised_tool_names()`.
#[tokio::test]
async fn http_catalogue_matches_advertised_names() {
    let res = server().get("/api/mcp/tools").await;
    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    let tools = body.as_array().expect("list_tools must return an array");

    let http_names: std::collections::HashSet<&str> = tools
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();

    let expected: std::collections::HashSet<&str> = mcp::advertised_tool_names().into_iter().collect();

    let missing: Vec<_> = expected.difference(&http_names).copied().collect();
    assert!(
        missing.is_empty(),
        "tools in advertised_tool_names() not found in HTTP /mcp/tools response: {missing:?}"
    );

    let extra: Vec<_> = http_names.difference(&expected).copied().collect();
    assert!(
        extra.is_empty(),
        "tools in HTTP /mcp/tools response not in advertised_tool_names(): {extra:?}"
    );
}
