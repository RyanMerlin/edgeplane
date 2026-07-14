mod common;

use axum_test::TestServer;
use common::{mint_session_with_groups, seed_null_domain_mission, setup};
use edgeplane_tower::{build_app, AppConfig};

fn server(pool: sqlx::PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

fn bearer(token: &str) -> (axum::http::HeaderName, String) {
    (axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
}

fn server_with_admin_groups(pool: sqlx::PgPool, groups: &[&str]) -> TestServer {
    let config = AppConfig {
        admin_groups: groups.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    TestServer::new(build_app(pool, config))
}

// Group C (artifacts.rs / docs.rs) authz: create_artifact/update_artifact/publish_artifact
// and create_doc/update_doc/publish_doc previously authorized the mission's domain
// ONLY when it was `Some` (`if let Some(ref mid) = domain_id && !can_write_domain(...)`),
// so a domainless mission (NULL domain_id) skipped the check entirely — any
// authenticated caller could write. They now use the same explicit
// `Some => can_write_domain, None => require admin` match that
// delete_artifact/delete_doc/get_* already used. These tests prove a non-admin is
// denied on a NULL-domain mission, an admin is allowed, and the `Some` branch still
// denies cross-domain outsiders on a WITH-domain mission (regression check).

#[tokio::test]
async fn create_artifact_null_domain_denied_for_non_admin() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let null_mission = seed_null_domain_mission(&pool).await;
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .post("/api/artifacts")
        .add_header(h, v)
        .json(&serde_json::json!({ "mission_id": null_mission, "name": "x" }))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "non-admin must not write an artifact on a NULL-domain mission, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn create_artifact_null_domain_allowed_for_admin() {
    let Some((pool, _ctx)) = setup().await else {
        return;
    };
    let null_mission = seed_null_domain_mission(&pool).await;
    let token =
        mint_session_with_groups(&pool, "sub-admin", "admin@example.com", &["EdgePlane Admins"])
            .await;
    let (h, v) = bearer(&token);
    let res = server_with_admin_groups(pool, &["EdgePlane Admins"])
        .post("/api/artifacts")
        .add_header(h, v)
        .json(&serde_json::json!({ "mission_id": null_mission, "name": "x" }))
        .await;
    assert_eq!(
        res.status_code(),
        200,
        "admin must be allowed to write an artifact on a NULL-domain mission, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn create_doc_null_domain_denied_for_non_admin() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let null_mission = seed_null_domain_mission(&pool).await;
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .post("/api/docs")
        .add_header(h, v)
        .json(&serde_json::json!({ "mission_id": null_mission, "title": "x" }))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "non-admin must not write a doc on a NULL-domain mission, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn create_artifact_with_domain_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .post("/api/artifacts")
        .add_header(h, v)
        .json(&serde_json::json!({ "mission_id": ctx.mission_id, "name": "x" }))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "cross-domain outsider must still be denied on a WITH-domain mission, got {}",
        res.status_code()
    );
}

// Group E (runtime.rs / budgets.rs) authz: create_job trusted caller-supplied
// body.domain_id with no authorization before the INSERT; record_usage_batch trusted
// per-record domain_id with no authorization. Both now gate via
// `crate::routes::authz::authz_domain` before writing.

#[tokio::test]
async fn create_job_denied_for_unauthorized_domain() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .post("/api/runtime/jobs")
        .add_header(h, v)
        .json(&serde_json::json!({ "domain_id": ctx.domain_id }))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "outsider must not create a job in a domain it isn't authorized for, got {}",
        res.status_code()
    );
}

// A domainless job (empty domain_id) is admin-only, mirroring Group C's NULL-domain
// policy. A non-admin (even a domain owner) is denied; an admin is allowed.
#[tokio::test]
async fn create_job_empty_domain_denied_for_non_admin() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (h, v) = bearer(&ctx.owner_session_token);
    let res = server(pool)
        .post("/api/runtime/jobs")
        .add_header(h, v)
        .json(&serde_json::json!({ "domain_id": "" }))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "non-admin must not create a domainless job, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn create_job_empty_domain_allowed_for_admin() {
    let Some((pool, _ctx)) = setup().await else {
        return;
    };
    let admin = mint_session_with_groups(
        &pool,
        "admin-job@example.com",
        "admin-job@example.com",
        &["admins"],
    )
    .await;
    let (h, v) = bearer(&admin);
    let res = server_with_admin_groups(pool, &["admins"])
        .post("/api/runtime/jobs")
        .add_header(h, v)
        .json(&serde_json::json!({ "domain_id": "" }))
        .await;
    assert_eq!(
        res.status_code(),
        201,
        "admin must be allowed to create a domainless job, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn create_job_allowed_for_owner() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (h, v) = bearer(&ctx.owner_session_token);
    let res = server(pool)
        .post("/api/runtime/jobs")
        .add_header(h, v)
        .json(&serde_json::json!({ "domain_id": ctx.domain_id }))
        .await;
    assert_eq!(
        res.status_code(),
        201,
        "domain owner must be allowed to create a job in its own domain, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn record_usage_batch_denied_for_unauthorized_domain() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .post("/api/budgets/usage/batch")
        .add_header(h, v)
        .json(&serde_json::json!({
            "records": [
                { "runtime_kind": "test", "domain_id": ctx.domain_id }
            ]
        }))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "outsider must not attribute usage to a domain it isn't authorized for, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn record_usage_batch_allowed_for_owner_and_null() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (h, v) = bearer(&ctx.owner_session_token);
    let res = server(pool)
        .post("/api/budgets/usage/batch")
        .add_header(h, v)
        .json(&serde_json::json!({
            "records": [
                { "runtime_kind": "test", "domain_id": ctx.domain_id },
                { "runtime_kind": "test", "domain_id": null }
            ]
        }))
        .await;
    assert_eq!(
        res.status_code(),
        200,
        "domain owner must be allowed to record usage for its own domain plus unattributed rows, got {}",
        res.status_code()
    );
}
