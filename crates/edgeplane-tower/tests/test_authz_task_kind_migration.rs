//! Migration 0014 (task/meshtask unification) authz regression tests for the
//! three new/changed guards introduced while retiring `tasks.rs::domain_access`:
//!
//!   - `authz_domain_readable` (list_tasks/get_task/list_overlaps): public-domain
//!     read bypass, including the case-insensitive `visibility` fix.
//!   - `authz_domain` (create_task/update_task): the deliberate widening that lets
//!     a `domain_scope` principal (not listed in owners/contributors) write.
//!   - `authz_domain_owner` (delete_task): the deliberately-NOT-widened owner-only
//!     bar — contributors must still be denied.
//!
//! Principal construction (mint_session for a no-standing outsider,
//! enroll-an-agent for a domain_scope principal, ctx.member_sa_token for a
//! contributor) follows the patterns already established in `test_authz.rs`.

mod common;

use axum_test::TestServer;
use common::{mint_session, seed_assigned_task, seed_domain, seed_mission_in_domain, setup};
use edgeplane_tower::{AppConfig, build_app};

fn server(pool: sqlx::PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

fn bearer(token: &str) -> (axum::http::HeaderName, String) {
    (axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
}

/// Enroll an agent (as owner session) and return the enrolled agent_token.
/// The resulting principal's `domain_scope` is exactly `[domain_id]` — never
/// listed in any `owners`/`contributors` CSV, so it isolates the
/// `domain_scope` authorization path from the CSV-membership path. Mirrors
/// `test_authz.rs`'s helper of the same name (duplicated per this crate's
/// existing per-file convention for integration-test-only helpers).
async fn enroll_agent_token(s: &TestServer, domain_id: &str, session_token: &str) -> String {
    let res = s
        .post(&format!("/api/work/domains/{domain_id}/agents/enroll"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {session_token}"),
        )
        .json(&serde_json::json!({"runtime_kind": "test"}))
        .await;
    assert_eq!(res.status_code(), 201, "enroll failed: {}", res.text());
    let body: serde_json::Value = res.json();
    body["agent_token"].as_str().unwrap().to_string()
}

// ── authz_domain_readable: public-domain read bypass ─────────────────────────

#[tokio::test]
async fn readable_public_domain_grants_list_tasks_to_no_standing_principal() {
    let Some((pool, _ctx)) = setup().await else {
        return;
    };
    let domain_id = seed_domain(&pool, "someone-else@example.com", "", "public").await;
    let mission_id = seed_mission_in_domain(&pool, &domain_id, "m-public-read").await;
    let _task_id =
        seed_assigned_task(&pool, &mission_id, &domain_id, "someone-else@example.com").await;

    // "erin" has no owner/contributor/admin/domain_scope standing anywhere.
    let token = mint_session(&pool, "erin", "erin@example.com").await;
    let (h, v) = bearer(&token);
    let res = server(pool)
        .get(&format!("/api/domains/{domain_id}/m/{mission_id}/t"))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        200,
        "public domain must grant list_tasks to a no-standing principal: {}",
        res.text()
    );
}

#[tokio::test]
async fn readable_private_domain_denies_no_standing_principal() {
    let Some((pool, _ctx)) = setup().await else {
        return;
    };
    let domain_id = seed_domain(&pool, "someone-else@example.com", "", "private").await;
    let mission_id = seed_mission_in_domain(&pool, &domain_id, "m-private-read").await;

    let token = mint_session(&pool, "erin", "erin@example.com").await;
    let (h, v) = bearer(&token);
    let res = server(pool)
        .get(&format!("/api/domains/{domain_id}/m/{mission_id}/t"))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "private domain must deny a no-standing principal: {}",
        res.text()
    );
}

#[tokio::test]
async fn readable_mixed_case_public_visibility_still_grants_access() {
    let Some((pool, _ctx)) = setup().await else {
        return;
    };
    // Regression: `visibility = 'Public'` (mixed case) must still be treated
    // as public — a real bug caught and fixed this session
    // (authz_domain_readable compares case-insensitively).
    let domain_id = seed_domain(&pool, "someone-else@example.com", "", "Public").await;
    let mission_id = seed_mission_in_domain(&pool, &domain_id, "m-mixed-case-public").await;
    let task_id =
        seed_assigned_task(&pool, &mission_id, &domain_id, "someone-else@example.com").await;

    let token = mint_session(&pool, "erin", "erin@example.com").await;
    let (h, v) = bearer(&token);
    let s = server(pool);

    let list_res = s
        .get(&format!("/api/domains/{domain_id}/m/{mission_id}/t"))
        .add_header(h.clone(), v.clone())
        .await;
    assert_eq!(
        list_res.status_code(),
        200,
        "visibility='Public' (mixed case) must still grant list_tasks: {}",
        list_res.text()
    );

    let get_res = s
        .get(&format!(
            "/api/domains/{domain_id}/m/{mission_id}/t/{task_id}"
        ))
        .add_header(h.clone(), v.clone())
        .await;
    assert_eq!(
        get_res.status_code(),
        200,
        "visibility='Public' (mixed case) must still grant get_task: {}",
        get_res.text()
    );

    let overlaps_res = s
        .get(&format!(
            "/api/domains/{domain_id}/m/{mission_id}/t/{task_id}/overlaps"
        ))
        .add_header(h, v)
        .await;
    assert_eq!(
        overlaps_res.status_code(),
        200,
        "visibility='Public' (mixed case) must still grant list_overlaps: {}",
        overlaps_res.text()
    );
}

// ── authz_domain: create_task/update_task domain_scope widening ─────────────

#[tokio::test]
async fn create_task_allowed_for_domain_scope_principal_not_in_owners_or_contributors() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    // Enrolled agent's domain_scope=[ctx.domain_id]; its subject
    // ("agent:<uuid>") never appears in ctx.domain_id's owners/contributors
    // CSV (setup() sets those to the owner email and the member SA only).
    let agent_token = enroll_agent_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    let res = s
        .post(&format!(
            "/api/domains/{}/m/{}/t",
            ctx.domain_id, ctx.mission_id
        ))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({"title": "domain-scope create"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "domain_scope principal (not owner/contributor) must be able to create_task \
         — this is the deliberate widening vs. the old domain_access(): {}",
        res.text()
    );
}

#[tokio::test]
async fn create_task_denied_for_domain_scope_principal_in_different_domain() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    // Agent is scoped to ctx.domain_id only.
    let agent_token = enroll_agent_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    // Target a mission in a DIFFERENT domain (ctx.other_domain_id has no
    // mission from setup() — seed one).
    let other_mission_id =
        seed_mission_in_domain(&pool, &ctx.other_domain_id, "m-other-domain-create").await;

    let res = s
        .post(&format!(
            "/api/domains/{}/m/{}/t",
            ctx.other_domain_id, other_mission_id
        ))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({"title": "cross-domain create"}))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "domain_scope principal scoped to a DIFFERENT domain must be denied create_task: {}",
        res.text()
    );
}

#[tokio::test]
async fn update_task_allowed_for_domain_scope_principal_not_in_owners_or_contributors() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id = seed_assigned_task(&pool, &ctx.mission_id, &ctx.domain_id, "harness").await;
    let s = server(pool.clone());
    let agent_token = enroll_agent_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    let res = s
        .patch(&format!(
            "/api/domains/{}/m/{}/t/{task_id}",
            ctx.domain_id, ctx.mission_id
        ))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({"status": "in_progress"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "domain_scope principal (not owner/contributor) must be able to update_task: {}",
        res.text()
    );
}

#[tokio::test]
async fn update_task_denied_for_domain_scope_principal_in_different_domain() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let other_mission_id =
        seed_mission_in_domain(&pool, &ctx.other_domain_id, "m-other-domain-update").await;
    let task_id =
        seed_assigned_task(&pool, &other_mission_id, &ctx.other_domain_id, "harness").await;
    let s = server(pool.clone());
    let agent_token = enroll_agent_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    let res = s
        .patch(&format!(
            "/api/domains/{}/m/{other_mission_id}/t/{task_id}",
            ctx.other_domain_id
        ))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({"status": "in_progress"}))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "domain_scope principal scoped to a DIFFERENT domain must be denied update_task: {}",
        res.text()
    );
}

// ── authz_domain_owner: delete_task — the strict bar that must NOT widen ────

#[tokio::test]
async fn delete_task_denied_for_contributor_not_owner() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id = seed_assigned_task(&pool, &ctx.mission_id, &ctx.domain_id, "harness").await;
    let s = server(pool);

    // ctx.member_sa_token is a contributor of ctx.domain_id (per setup()),
    // NOT an owner — authz_domain_owner must reject it (contributors excluded
    // by design, unlike authz_domain/authz_domain_readable).
    let res = s
        .delete(&format!(
            "/api/domains/{}/m/{}/t/{task_id}",
            ctx.domain_id, ctx.mission_id
        ))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "a contributor (not owner) must be denied delete_task — the strict bar must hold: {}",
        res.text()
    );
}

#[tokio::test]
async fn delete_task_allowed_for_domain_scope_principal_not_in_owners() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id = seed_assigned_task(&pool, &ctx.mission_id, &ctx.domain_id, "harness").await;
    let s = server(pool.clone());
    let agent_token = enroll_agent_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    let res = s
        .delete(&format!(
            "/api/domains/{}/m/{}/t/{task_id}",
            ctx.domain_id, ctx.mission_id
        ))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert!(
        res.status_code().is_success(),
        "domain_scope principal (not in owners CSV) must be able to delete_task — the \
         domain_scope widening applies here too, matching create/update/read: {}",
        res.text()
    );
}

#[tokio::test]
async fn delete_task_allowed_for_owner() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id = seed_assigned_task(&pool, &ctx.mission_id, &ctx.domain_id, "harness").await;
    let s = server(pool);
    let res = s
        .delete(&format!(
            "/api/domains/{}/m/{}/t/{task_id}",
            ctx.domain_id, ctx.mission_id
        ))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert!(
        res.status_code().is_success(),
        "the domain owner must still be able to delete_task: {}",
        res.text()
    );
}
