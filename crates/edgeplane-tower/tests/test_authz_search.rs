mod common;

use axum_test::TestServer;
use common::{mint_session, seed_doc, seed_domain, seed_mission_in_domain, seed_task_titled};
use edgeplane_tower::{build_app, AppConfig};

fn server(pool: sqlx::PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

fn bearer(token: &str) -> (axum::http::HeaderName, String) {
    (axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
}

// Group F (search.rs) authz: the non-admin readability filter used to do
// `LOWER(m.owners) LIKE '%subject%'` — a SUBSTRING test against a CSV field.
// A subject `alice` therefore matched an owners value of
// `alicexyz@example.com`, leaking tasks/docs/missions to a non-member whose
// name happened to be a prefix of a real owner. The fix replaces the LIKE
// membership check with `crate::auth::authorized_for`, an exact
// comma-separated-entry match. These tests prove exact membership both
// directions, plus that the public-domain path still works.

#[tokio::test]
async fn search_task_substring_owner_denied() {
    let Some((pool, _ctx)) = common::setup().await else {
        return;
    };
    let domain_id = seed_domain(&pool, "alicexyz@example.com", "", "private").await;
    let mission_id = seed_mission_in_domain(&pool, &domain_id, "m-search-task").await;
    let _task_id = seed_task_titled(&pool, &mission_id, "zebra crossing task").await;

    // "alice" is a substring of the real owner "alicexyz@example.com" but is
    // not itself a member — must NOT see the task.
    let token = mint_session(&pool, "alice", "alice@example.com").await;
    let (h, v) = bearer(&token);
    let res = server(pool)
        .get("/api/search/tasks?q=zebra")
        .add_header(h, v)
        .await;
    assert_eq!(res.status_code(), 200);
    let body: serde_json::Value = res.json();
    let results = body["results"].as_array().expect("results array");
    assert!(
        results.iter().all(|r| r["title"] != "zebra crossing task"),
        "substring subject 'alice' must NOT see a task owned by 'alicexyz@example.com', got {results:?}"
    );
}

#[tokio::test]
async fn search_task_exact_owner_allowed() {
    let Some((pool, _ctx)) = common::setup().await else {
        return;
    };
    let domain_id = seed_domain(&pool, "alicexyz@example.com", "", "private").await;
    let mission_id = seed_mission_in_domain(&pool, &domain_id, "m-search-task-2").await;
    let _task_id = seed_task_titled(&pool, &mission_id, "zebra crossing task").await;

    let token = mint_session(&pool, "alicexyz@example.com", "alicexyz@example.com").await;
    let (h, v) = bearer(&token);
    let res = server(pool)
        .get("/api/search/tasks?q=zebra")
        .add_header(h, v)
        .await;
    assert_eq!(res.status_code(), 200);
    let body: serde_json::Value = res.json();
    let results = body["results"].as_array().expect("results array");
    assert!(
        results.iter().any(|r| r["title"] == "zebra crossing task"),
        "exact owner 'alicexyz@example.com' must see its own task, got {results:?}"
    );
}

#[tokio::test]
async fn search_docs_substring_owner_denied() {
    let Some((pool, _ctx)) = common::setup().await else {
        return;
    };
    let domain_id = seed_domain(&pool, "alicexyz@example.com", "", "private").await;
    let mission_id = seed_mission_in_domain(&pool, &domain_id, "m-search-doc").await;
    let _doc_id = seed_doc(&pool, &mission_id, "walrus doc title", "body text").await;

    let token = mint_session(&pool, "alice", "alice@example.com").await;
    let (h, v) = bearer(&token);
    let res = server(pool)
        .get("/api/search/docs?q=walrus")
        .add_header(h, v)
        .await;
    assert_eq!(res.status_code(), 200);
    let body: serde_json::Value = res.json();
    let results = body["results"].as_array().expect("results array");
    assert!(
        results.iter().all(|r| r["title"] != "walrus doc title"),
        "substring subject 'alice' must NOT see a doc owned by 'alicexyz@example.com', got {results:?}"
    );
}

#[tokio::test]
async fn search_docs_exact_owner_allowed() {
    let Some((pool, _ctx)) = common::setup().await else {
        return;
    };
    let domain_id = seed_domain(&pool, "alicexyz@example.com", "", "private").await;
    let mission_id = seed_mission_in_domain(&pool, &domain_id, "m-search-doc-2").await;
    let _doc_id = seed_doc(&pool, &mission_id, "walrus doc title", "body text").await;

    let token = mint_session(&pool, "alicexyz@example.com", "alicexyz@example.com").await;
    let (h, v) = bearer(&token);
    let res = server(pool)
        .get("/api/search/docs?q=walrus")
        .add_header(h, v)
        .await;
    assert_eq!(res.status_code(), 200);
    let body: serde_json::Value = res.json();
    let results = body["results"].as_array().expect("results array");
    assert!(
        results.iter().any(|r| r["title"] == "walrus doc title"),
        "exact owner 'alicexyz@example.com' must see its own doc, got {results:?}"
    );
}

#[tokio::test]
async fn search_missions_substring_owner_denied() {
    let Some((pool, _ctx)) = common::setup().await else {
        return;
    };
    let domain_id = seed_domain(&pool, "alicexyz@example.com", "", "private").await;
    let _mission_id = seed_mission_in_domain(&pool, &domain_id, "gorilla-mission").await;

    let token = mint_session(&pool, "alice", "alice@example.com").await;
    let (h, v) = bearer(&token);
    let res = server(pool)
        .get("/api/search/missions?q=gorilla")
        .add_header(h, v)
        .await;
    assert_eq!(res.status_code(), 200);
    let body: serde_json::Value = res.json();
    let results = body["results"].as_array().expect("results array");
    assert!(
        results.iter().all(|r| r["name"] != "gorilla-mission"),
        "substring subject 'alice' must NOT see a mission whose domain is owned by \
         'alicexyz@example.com', got {results:?}"
    );
}

#[tokio::test]
async fn search_missions_exact_owner_allowed() {
    let Some((pool, _ctx)) = common::setup().await else {
        return;
    };
    let domain_id = seed_domain(&pool, "alicexyz@example.com", "", "private").await;
    let _mission_id = seed_mission_in_domain(&pool, &domain_id, "gorilla-mission").await;

    let token = mint_session(&pool, "alicexyz@example.com", "alicexyz@example.com").await;
    let (h, v) = bearer(&token);
    let res = server(pool)
        .get("/api/search/missions?q=gorilla")
        .add_header(h, v)
        .await;
    assert_eq!(res.status_code(), 200);
    let body: serde_json::Value = res.json();
    let results = body["results"].as_array().expect("results array");
    assert!(
        results.iter().any(|r| r["name"] == "gorilla-mission"),
        "exact owner 'alicexyz@example.com' must see its own mission, got {results:?}"
    );
}

#[tokio::test]
async fn search_public_mission_visible_to_non_member() {
    let Some((pool, _ctx)) = common::setup().await else {
        return;
    };
    let domain_id = seed_domain(&pool, "someoneelse@example.com", "", "public").await;
    let mission_id = seed_mission_in_domain(&pool, &domain_id, "public-otter-mission").await;
    let _task_id = seed_task_titled(&pool, &mission_id, "public otter task").await;

    // "bob" is not an owner or contributor of the domain, and not a substring
    // of one either — proves the public-visibility path (not a leftover
    // substring match) is what grants access.
    let token = mint_session(&pool, "bob", "bob@example.com").await;
    let (h, v) = bearer(&token);

    let mission_res = server(pool.clone())
        .get("/api/search/missions?q=otter")
        .add_header(h.clone(), v.clone())
        .await;
    assert_eq!(mission_res.status_code(), 200);
    let mission_body: serde_json::Value = mission_res.json();
    let mission_results = mission_body["results"].as_array().expect("results array");
    assert!(
        mission_results
            .iter()
            .any(|r| r["name"] == "public-otter-mission"),
        "non-member must see a mission in a public domain, got {mission_results:?}"
    );

    let task_res = server(pool)
        .get("/api/search/tasks?q=otter")
        .add_header(h, v)
        .await;
    assert_eq!(task_res.status_code(), 200);
    let task_body: serde_json::Value = task_res.json();
    let task_results = task_body["results"].as_array().expect("results array");
    assert!(
        task_results.iter().any(|r| r["title"] == "public otter task"),
        "non-member must see a task in a public domain, got {task_results:?}"
    );
}
