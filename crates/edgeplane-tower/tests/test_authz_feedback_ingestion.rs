mod common;

use axum_test::TestServer;
use common::{seed_ingestion_job, setup};
use edgeplane_tower::{AppConfig, build_app};

fn server(pool: sqlx::PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

fn bearer(token: &str) -> (axum::http::HeaderName, String) {
    (axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
}

// Broken-access-control remediation for two read surfaces the Group A enumeration
// missed (surfaced by the 2026-07-10 dual-review; plan Groups E + G, tracked in
// docs/plans/2026-07-10-authz-hardening.md on the fix/authz-hardening branch):
//
//  * feedback.rs::{list_feedback, feedback_summary} filtered on the CALLER-SUPPLIED
//    `domain_id` with the principal ignored — any authenticated caller could read
//    any domain's feedback (confused deputy).
//  * ingestion.rs::{list_jobs, get_job} had no gate; list_jobs with no mission_id
//    dumped every tenant's jobs.
//
// The gates resolve the owning domain (feedback: the query domain_id; ingestion:
// the mission's domain) and require caller membership. These tests prove an
// owner reads their own domain/mission while a cross-domain outsider gets 403.

// ---- feedback.rs (Group E) ----

#[tokio::test]
async fn list_feedback_allows_domain_member() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (h, v) = bearer(&ctx.owner_session_token);
    let res = server(pool)
        .get(&format!("/api/feedback?domain_id={}", ctx.domain_id))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        200,
        "domain owner must be allowed to list own-domain feedback, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn list_feedback_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .get(&format!("/api/feedback?domain_id={}", ctx.domain_id))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "cross-domain outsider must be denied the feedback list, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn feedback_summary_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .get(&format!(
            "/api/feedback/summary?domain_id={}",
            ctx.domain_id
        ))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "cross-domain outsider must be denied the feedback summary, got {}",
        res.status_code()
    );
}

// ---- ingestion.rs (Group G) ----

#[tokio::test]
async fn list_jobs_allows_domain_member() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (h, v) = bearer(&ctx.owner_session_token);
    let res = server(pool)
        .get(&format!("/api/ingest/jobs?mission_id={}", ctx.mission_id))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        200,
        "mission-domain owner must list own-mission jobs, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn list_jobs_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .get(&format!("/api/ingest/jobs?mission_id={}", ctx.mission_id))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "cross-domain outsider must be denied the mission's job list, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn list_jobs_requires_mission_id_for_non_admin() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // owner_session_token is a domain owner but NOT a global admin — the no-filter
    // all-missions dump must be refused (422), not returned.
    let (h, v) = bearer(&ctx.owner_session_token);
    let res = server(pool).get("/api/ingest/jobs").add_header(h, v).await;
    assert_eq!(
        res.status_code(),
        422,
        "non-admin must not dump all missions' jobs; mission_id required, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn get_job_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let job_id = seed_ingestion_job(&pool, &ctx.mission_id).await;
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .get(&format!("/api/ingest/jobs/{job_id}"))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "cross-domain outsider must be denied reading another mission's job, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn get_job_allows_domain_member() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let job_id = seed_ingestion_job(&pool, &ctx.mission_id).await;
    let (h, v) = bearer(&ctx.owner_session_token);
    let res = server(pool)
        .get(&format!("/api/ingest/jobs/{job_id}"))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        200,
        "mission-domain owner must read own-mission job, got {}",
        res.status_code()
    );
}
