//! Regression test: `POST /budgets` and `GET /budgets` must actually work.
//! `BudgetPolicyCreate`/`row_to_policy` (routes/budgets.rs) reference
//! `token_hard_cap`/`token_soft_cap` columns that never existed on
//! `budgetpolicy` (`crates/edgeplane-tower/migrations/0001_initial_schema.sql`
//! only has `hard_cap_cents`/`soft_cap_cents`) — inherited from the pre-fork
//! MissionControl codebase, present since the very first commit. Every
//! POST/GET call has always failed: POST 500'd on "column does not exist"
//! at the INSERT; GET would have panicked on `Row::get` for the missing
//! columns. Fixed via migration 0013 (additive, nullable columns).

mod common;

use axum_test::TestServer;
use common::setup;
use edgeplane_tower::{AppConfig, build_app};
use sqlx::PgPool;

fn server(pool: PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

#[tokio::test]
async fn create_and_list_budget_policy_round_trips_token_caps() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };

    let s = server(pool);
    let create_res = s
        .post("/api/budgets")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "scope_type": "domain",
            "scope_id": ctx.domain_id,
            "window_type": "monthly",
            "hard_cap_cents": 10000,
            "soft_cap_cents": 8000,
            "token_hard_cap": 500000,
            "token_soft_cap": 400000,
        }))
        .await;

    create_res.assert_status(axum::http::StatusCode::CREATED);
    let created: serde_json::Value = create_res.json();
    assert_eq!(
        created["token_hard_cap"],
        serde_json::json!(500000),
        "response body: {created}"
    );
    assert_eq!(
        created["token_soft_cap"],
        serde_json::json!(400000),
        "response body: {created}"
    );

    let list_res = s
        .get("/api/budgets")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;

    list_res.assert_status_ok();
    let listed: serde_json::Value = list_res.json();
    let policies = listed.as_array().expect("result must be array");
    assert_eq!(policies.len(), 1, "response body: {listed}");
    assert_eq!(policies[0]["token_hard_cap"], serde_json::json!(500000));
    assert_eq!(policies[0]["token_soft_cap"], serde_json::json!(400000));
}
