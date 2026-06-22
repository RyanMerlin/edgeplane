mod common;

use axum_test::TestServer;
use common::setup;
use edgeplane_tower::{AppConfig, build_app};
use sqlx::Row;

fn server(pool: sqlx::PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

/// A server running on a real HTTP port; required for WS upgrade requests.
fn http_server(pool: sqlx::PgPool) -> TestServer {
    TestServer::builder()
        .http_transport()
        .build(build_app(pool, AppConfig::default()))
}

#[tokio::test]
async fn harness_skips_without_db() {
    // Compiles the harness; no-op unless TEST_DATABASE_URL is set.
    let _ = common::setup().await;
}

#[tokio::test]
async fn create_task_denied_for_outsider_sa() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let res = s
        .post(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.outsider_sa_token),
        )
        .json(&serde_json::json!({ "title": "pwn" }))
        .await;
    assert_eq!(res.status_code(), 403);
}

#[tokio::test]
async fn create_task_allowed_for_owner_session() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let res = s
        .post(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({ "title": "legit" }))
        .await;
    assert_eq!(res.status_code(), 201);
}

#[tokio::test]
async fn create_task_allowed_for_member_sa_contributor() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let res = s
        .post(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({ "title": "ok" }))
        .await;
    assert_eq!(res.status_code(), 201);
}

#[tokio::test]
async fn mcp_submit_mesh_task_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.outsider_sa_token),
        )
        .json(&serde_json::json!({
            "tool": "submit_mesh_task",
            "args": { "mission_id": ctx.mission_id, "title": "pwn" }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(body["error"], "forbidden");
}

#[tokio::test]
async fn domain_stream_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // Use a real HTTP port so WebSocketUpgrade extraction can find hyper's
    // OnUpgrade extension. Our authz guard fires before on_upgrade and returns
    // 403; the connection is never actually upgraded.
    let s = http_server(pool);
    let res = s
        .get(&format!("/api/work/domains/{}/stream", ctx.other_domain_id))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.outsider_sa_token),
        )
        .add_header(axum::http::header::CONNECTION, "upgrade")
        .add_header(axum::http::header::UPGRADE, "websocket")
        .add_header(axum::http::header::SEC_WEBSOCKET_VERSION, "13")
        .add_header(axum::http::header::SEC_WEBSOCKET_KEY, "dGhlIHNhbXBsZSBub25jZQ==")
        .await;
    assert_eq!(res.status_code(), 403);
}

#[tokio::test]
async fn agent_cannot_complete_unassigned_task() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // Seed a task claimed by "agent-A" — member_sa is a domain contributor so
    // the domain guard passes, but it is NOT agent-A, so the owner guard fires.
    let task_id = common::seed_claimed_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "agent-A",
    )
    .await;
    let s = server(pool.clone());

    // A domain-member SA that is not the claimer must get 403.
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert_eq!(res.status_code(), 403);

    // A full-trust session owner can complete it.
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({}))
        .await;
    // 200 means finished; waiting_review (200) is also acceptable if gates exist.
    assert!(
        res.status_code().is_success(),
        "owner session should complete task, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn mcp_get_artifact_download_url_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let artifact_id = common::seed_artifact(&pool, &ctx.mission_id).await;
    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.outsider_sa_token),
        )
        .json(&serde_json::json!({
            "tool": "get_artifact_download_url",
            "args": { "artifact_id": artifact_id }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(body["error"], "forbidden");
}

#[tokio::test]
async fn global_sse_denied_for_non_admin() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let res = s
        .get("/api/sse")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert_eq!(res.status_code(), 403);
}

// ── T5: attribution — claim/progress attributed to authenticated agent ─────────

#[tokio::test]
async fn agent_token_attributes_progress_to_self() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // Enroll an agent and seed a task it owns.
    let (agent_id, agent_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let task_id =
        common::seed_claimed_task(&pool, &ctx.mission_id, &ctx.domain_id, &agent_id).await;

    // POST progress with the agent's token — no agent_id in body.
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/progress"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({"event_type": "status", "summary": "working"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "progress should succeed: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    // The agent_id in the response must equal the enrolled agent_id (not empty, not the full subject).
    assert_eq!(
        body["agent_id"].as_str().unwrap_or(""),
        agent_id,
        "progress agent_id must be attributed to the caller's agent_id"
    );
}

#[tokio::test]
async fn agent_cannot_spoof_claim_agent_id() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // Enroll two agents.
    let (agent_a_id, agent_a_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let (agent_b_id, _) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    // Create a ready task in the domain.
    let task_id = {
        use uuid::Uuid;
        let tid = format!("task-{}", Uuid::new_v4().simple());
        sqlx::query(
            "INSERT INTO meshtask \
             (id, mission_id, domain_id, title, description, input_json, claim_policy, \
              depends_on, produces, consumes, required_capabilities, \
              status, priority, version_counter, created_by_subject, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, 'spoof-test', '', '{}', 'any', '[]', '{}', '{}', '[]', \
                     'ready', 0, 1, 'harness', now(), now())",
        )
        .bind(&tid)
        .bind(&ctx.mission_id)
        .bind(&ctx.domain_id)
        .execute(&pool)
        .await
        .expect("insert task");
        tid
    };

    // Agent-A tries to claim the task on behalf of Agent-B (agent_id spoofing).
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/claim"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .json(&serde_json::json!({"agent_id": agent_b_id}))
        .await;
    assert!(
        res.status_code().is_success(),
        "claim should succeed (agent-A is a domain member): {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    // The claimed_by_agent_id must be agent-A's own id, not the spoofed agent-B.
    let claimed = body["claimed_by_agent_id"].as_str().unwrap_or("");
    assert_eq!(
        claimed, agent_a_id,
        "claim must be attributed to the caller (agent-A), not the spoofed agent-B"
    );
    assert_ne!(
        claimed, agent_b_id,
        "spoofed agent-B must not appear as claimer"
    );
}

// ── T4: per-agent JWT — mint endpoint + enrollment ────────────────────────────

/// Enroll an agent (as owner session) and return the enrolled agent_id +
/// agent_token from the response.
async fn enroll_and_get_token(
    s: &axum_test::TestServer,
    domain_id: &str,
    session_token: &str,
) -> (String, String) {
    let res = s
        .post(&format!("/api/work/domains/{domain_id}/agents/enroll"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {session_token}"),
        )
        .json(&serde_json::json!({"runtime_kind": "test"}))
        .await;
    assert_eq!(
        res.status_code(),
        201,
        "enroll failed: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    let agent_id = body["id"].as_str().unwrap().to_string();
    let agent_token = body["agent_token"].as_str().unwrap().to_string();
    (agent_id, agent_token)
}

#[tokio::test]
async fn agent_cannot_mint_peer_token() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);

    // Enroll two agents via the owner session.
    let (agent_a_id, agent_a_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let (agent_b_id, _agent_b_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    // Agent-A must NOT be able to mint a token for agent-B (peer impersonation).
    let res = s
        .post(&format!("/api/work/agents/{agent_b_id}/token"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .json(&serde_json::json!({}))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "agent {agent_a_id} should not mint token for {agent_b_id}: {}",
        res.text()
    );
}

#[tokio::test]
async fn enrolled_agent_token_denied_in_foreign_domain() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // Use a real HTTP port so WebSocketUpgrade extraction can proceed.
    let s = http_server(pool);

    // Enroll an agent into domain A.
    let (_agent_id, agent_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    // The agent's token is scoped to domain A — accessing domain B's stream must 403.
    let res = s
        .get(&format!("/api/work/domains/{}/stream", ctx.other_domain_id))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .add_header(axum::http::header::CONNECTION, "upgrade")
        .add_header(axum::http::header::UPGRADE, "websocket")
        .add_header(axum::http::header::SEC_WEBSOCKET_VERSION, "13")
        .add_header(axum::http::header::SEC_WEBSOCKET_KEY, "dGhlIHNhbXBsZSBub25jZQ==")
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "agent token should not access foreign domain: {}",
        res.text()
    );
}

#[tokio::test]
async fn full_trust_session_can_mint_agent_token() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);

    // Enroll an agent via the owner session (to get the agent_id).
    let (agent_id, _) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    // A full-trust session should be able to re-mint a token for the agent.
    let res = s
        .post(&format!("/api/work/agents/{agent_id}/token"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert_eq!(
        res.status_code(),
        200,
        "full-trust session should mint agent token: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    assert!(
        body["agent_token"].as_str().is_some(),
        "response must contain agent_token"
    );
    assert_eq!(body["expires_in"], 12 * 3600);
}

// ── Seam-4 / red-team fixes: read-side cross-domain deny ─────────────────────

#[tokio::test]
async fn mcp_get_domain_northstar_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    // Outsider requests the OTHER domain's northstar — must be forbidden.
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.outsider_sa_token),
        )
        .json(&serde_json::json!({
            "tool": "get_domain_northstar",
            "args": { "domain_id": ctx.other_domain_id }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["error"], "forbidden",
        "outsider must not read foreign domain northstar: {body}"
    );
}

#[tokio::test]
async fn mcp_resolve_publish_plan_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.outsider_sa_token),
        )
        .json(&serde_json::json!({
            "tool": "resolve_publish_plan",
            "args": { "domain_id": ctx.other_domain_id, "entity_kind": "task" }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["error"], "forbidden",
        "outsider must not read foreign domain publish plan: {body}"
    );
}

#[tokio::test]
async fn mcp_list_mesh_tasks_requires_mission_id() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    // Omitting mission_id should now return an error.
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "list_mesh_tasks",
            "args": {}
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert!(
        body["ok"] == false || body["error"].is_string(),
        "list_mesh_tasks without mission_id must return an error: {body}"
    );
}

#[tokio::test]
async fn mcp_list_mesh_tasks_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.outsider_sa_token),
        )
        .json(&serde_json::json!({
            "tool": "list_mesh_tasks",
            "args": { "mission_id": ctx.mission_id }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["error"], "forbidden",
        "outsider must not list tasks in foreign mission: {body}"
    );
}

#[tokio::test]
async fn mcp_get_mesh_task_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // Seed a task in our domain; the outsider must not be able to read it.
    let task_id = common::seed_ready_task(&pool, &ctx.mission_id, &ctx.domain_id).await;
    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.outsider_sa_token),
        )
        .json(&serde_json::json!({
            "tool": "get_mesh_task",
            "args": { "task_id": task_id }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["error"], "forbidden",
        "outsider must not read foreign domain task: {body}"
    );
}

#[tokio::test]
async fn mcp_list_mesh_messages_broadcast_scoped_to_domain() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // Seed an agent in domain A with a known id (so we can query it).
    let agent_a_id = format!("agent-a-{}", uuid::Uuid::new_v4().simple());
    common::seed_agent(&pool, &ctx.domain_id, &agent_a_id).await;
    // Seed a broadcast in OTHER domain (domain B) — domain A's agent must NOT see it.
    common::seed_mesh_message(&pool, &ctx.other_domain_id, "agent-b", None).await;
    // Seed a broadcast in OUR domain (domain A) — domain A's agent SHOULD see it.
    common::seed_mesh_message(&pool, &ctx.domain_id, "agent-b", None).await;

    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "list_mesh_messages",
            "args": { "agent_id": agent_a_id, "limit": 100 }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(body["ok"], true, "list_mesh_messages failed: {body}");
    let messages = body["result"].as_array().expect("result must be array");
    // None of the returned messages should belong to the other domain.
    for msg in messages {
        assert_ne!(
            msg["domain_id"].as_str().unwrap_or(""),
            ctx.other_domain_id.as_str(),
            "domain B broadcast must not appear in domain A's message list"
        );
    }
    // At least one message from domain A should appear.
    let domain_a_msgs: Vec<_> = messages
        .iter()
        .filter(|m| m["domain_id"] == ctx.domain_id)
        .collect();
    assert!(
        !domain_a_msgs.is_empty(),
        "domain A broadcast should appear in message list"
    );
}

// ── IDOR: per-task owner checks ───────────────────────────────────────────────

#[tokio::test]
async fn domain_peer_cannot_unblock_foreign_task() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // Seed a task claimed by "agent-A"; domain member (member_sa) is NOT the claimer.
    let task_id =
        common::seed_claimed_task(&pool, &ctx.mission_id, &ctx.domain_id, "agent-A").await;
    // Mark it blocked so unblock makes sense.
    sqlx::query("UPDATE meshtask SET status='blocked' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .expect("set blocked");

    let s = server(pool);
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/unblock"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "domain peer must not unblock another agent's task: {}",
        res.text()
    );
}

#[tokio::test]
async fn mcp_progress_mesh_task_denied_for_non_owner() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    // Enroll agent A (the real claimer) and agent B (the non-owner).
    let (agent_a_id, _agent_a_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let (_agent_b_id, agent_b_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let task_id =
        common::seed_claimed_task(&pool, &ctx.mission_id, &ctx.domain_id, &agent_a_id).await;

    // Agent B (domain peer, not the claimer) tries to post progress — must fail.
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_b_token}"),
        )
        .json(&serde_json::json!({
            "tool": "progress_mesh_task",
            "args": { "task_id": task_id, "event_type": "status" }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["ok"], false,
        "non-owner must not post progress on another agent's task: {body}"
    );
}

#[tokio::test]
async fn domain_peer_cannot_create_gate_on_foreign_task() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // Seed a task claimed by "agent-A"; domain member is NOT the claimer.
    let task_id =
        common::seed_claimed_task(&pool, &ctx.mission_id, &ctx.domain_id, "agent-A").await;
    let s = server(pool);
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/gates"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({
            "gate_type": "review",
            "required_approvals": "1"
        }))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "domain peer must not attach gate to another agent's task: {}",
        res.text()
    );
}

// ── send_mesh_message sender anti-spoof ──────────────────────────────────────

#[tokio::test]
async fn send_mesh_message_spoof_rejected_for_restricted_caller() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // Enroll agent A; it will try to send as "agent-B" (spoof).
    let (agent_a_id, agent_a_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .json(&serde_json::json!({
            "tool": "send_mesh_message",
            "args": {
                "domain_id": ctx.domain_id,
                "sender_agent_id": "agent-B-spoof",
                "content": "hello"
            }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(body["ok"], true, "send should succeed: {body}");

    // Verify the persisted from_agent_id is agent_a_id, not the spoofed value.
    let from: String = sqlx::query_scalar(
        "SELECT from_agent_id FROM meshmessage WHERE domain_id=$1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&ctx.domain_id)
    .fetch_one(&pool)
    .await
    .expect("fetch message");
    assert_eq!(
        from, agent_a_id,
        "from_agent_id must be the caller's own id, not the spoofed value"
    );
}

// ── #61: get_overlap_suggestions authz ───────────────────────────────────────

/// An outsider calling get_overlap_suggestions on a task from a foreign domain
/// must receive a "forbidden" error, not the data.
#[tokio::test]
async fn mcp_get_overlap_suggestions_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // Seed a workspace task + overlap suggestion in ctx.domain's mission.
    let task_id = common::seed_task(&pool, &ctx.mission_id).await;
    let _ = common::seed_overlap_suggestion(&pool, task_id).await;

    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.outsider_sa_token),
        )
        .json(&serde_json::json!({
            "tool": "get_overlap_suggestions",
            "args": { "task_id": task_id }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["error"], "forbidden",
        "outsider must not read overlap suggestions from foreign domain task: {body}"
    );
}

/// An authorized caller (domain owner) can retrieve overlap suggestions.
#[tokio::test]
async fn mcp_get_overlap_suggestions_allowed_for_owner() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id = common::seed_task(&pool, &ctx.mission_id).await;
    let _ = common::seed_overlap_suggestion(&pool, task_id).await;

    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "get_overlap_suggestions",
            "args": { "task_id": task_id }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["ok"], true,
        "owner must be able to read overlap suggestions: {body}"
    );
    let results = body["result"].as_array().expect("result must be array");
    assert!(
        !results.is_empty(),
        "at least one overlap suggestion must be returned"
    );
}

// ── #61: agent self-identity tests ───────────────────────────────────────────

/// An enrolled agent may not heartbeat a different agent's id (peer heartbeat).
#[tokio::test]
async fn agent_cannot_heartbeat_peer() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);

    let (agent_a_id, agent_a_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let (agent_b_id, _agent_b_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    // Agent A tries to heartbeat Agent B — must be denied.
    let res = s
        .post(&format!("/api/work/agents/{agent_b_id}/heartbeat"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "agent {agent_a_id} must not heartbeat peer {agent_b_id}: {}",
        res.text()
    );

    // Agent A can heartbeat its own id.
    let res = s
        .post(&format!("/api/work/agents/{agent_a_id}/heartbeat"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .await;
    assert!(
        res.status_code().is_success(),
        "agent {agent_a_id} must be able to heartbeat itself: {}",
        res.text()
    );
}

/// An enrolled agent may not set status on a different agent's id.
#[tokio::test]
async fn agent_cannot_set_peer_status() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);

    let (agent_a_id, agent_a_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let (agent_b_id, _agent_b_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    // Agent A tries to set Agent B's status — must be denied.
    let res = s
        .post(&format!("/api/work/agents/{agent_b_id}/status"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .add_query_params([("status", "idle")])
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "agent {agent_a_id} must not set status on peer {agent_b_id}: {}",
        res.text()
    );

    // Agent A can set its own status.
    let res = s
        .post(&format!("/api/work/agents/{agent_a_id}/status"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .add_query_params([("status", "idle")])
        .await;
    assert!(
        res.status_code().is_success(),
        "agent {agent_a_id} must be able to set its own status: {}",
        res.text()
    );
}

/// An enrolled agent may not update a different agent's profile.
#[tokio::test]
async fn agent_cannot_update_peer_profile() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);

    let (agent_a_id, agent_a_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let (agent_b_id, _agent_b_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    // Agent A tries to update Agent B's profile — must be denied.
    let res = s
        .patch(&format!("/api/work/agents/{agent_b_id}/profile"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .json(&serde_json::json!({}))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "agent {agent_a_id} must not update profile of peer {agent_b_id}: {}",
        res.text()
    );

    // Full-trust session can update any agent's profile.
    let res = s
        .patch(&format!("/api/work/agents/{agent_b_id}/profile"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(
        res.status_code().is_success(),
        "full-trust session must be able to update any agent profile: {}",
        res.text()
    );
}

// ── DELETE /runtime/nodes/{node_id} ──────────────────────────────────────────

/// DELETE on an unknown node_id must return 404.
#[tokio::test]
async fn delete_node_404_for_unknown_node() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let res = s
        .delete("/api/runtime/nodes/no-such-node-id-ever")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert_eq!(res.status_code(), 404, "unknown node: {}", res.text());
}

/// DELETE by a non-owner must return 403.
#[tokio::test]
async fn delete_node_403_for_non_owner() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let node_name = format!("node-403-{}", uuid::Uuid::new_v4().simple());
    // Node is owned by ctx.owner_session_token's subject.
    let owner_email = {
        use edgeplane_tower::auth::hash_token;
        let hash = hash_token(&ctx.owner_session_token);
        sqlx::query_scalar::<_, String>(
            "SELECT subject FROM usersession WHERE token_hash = $1",
        )
        .bind(&hash)
        .fetch_one(&pool)
        .await
        .expect("find owner subject")
    };
    let node_id = common::seed_runtime_node(&pool, &owner_email, &node_name).await;
    let s = server(pool);

    // outsider_sa_token does not own this node.
    let res = s
        .delete(&format!("/api/runtime/nodes/{node_id}"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.outsider_sa_token),
        )
        .await;
    assert_eq!(res.status_code(), 403, "non-owner must be denied: {}", res.text());
}

/// DELETE without ?force=true when agents are assigned must return 409.
#[tokio::test]
async fn delete_node_409_with_assigned_agents_no_force() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let owner_email = {
        use edgeplane_tower::auth::hash_token;
        let hash = hash_token(&ctx.owner_session_token);
        sqlx::query_scalar::<_, String>(
            "SELECT subject FROM usersession WHERE token_hash = $1",
        )
        .bind(&hash)
        .fetch_one(&pool)
        .await
        .expect("find owner subject")
    };
    let node_name = format!("node-409-{}", uuid::Uuid::new_v4().simple());
    let node_id = common::seed_runtime_node(&pool, &owner_email, &node_name).await;
    let _agent_id = common::seed_node_agent(&pool, &ctx.domain_id, &node_id).await;
    let s = server(pool);

    let res = s
        .delete(&format!("/api/runtime/nodes/{node_id}"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "assigned agents without force must be rejected: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    assert!(
        body["assigned_agents"].as_i64().unwrap_or(0) >= 1,
        "response must report assigned_agents count"
    );
}

/// DELETE with ?force=true detaches agents, revokes tokens, and removes the node.
#[tokio::test]
async fn delete_node_success_with_force_detaches_agents_and_revokes_tokens() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let owner_email = {
        use edgeplane_tower::auth::hash_token;
        let hash = hash_token(&ctx.owner_session_token);
        sqlx::query_scalar::<_, String>(
            "SELECT subject FROM usersession WHERE token_hash = $1",
        )
        .bind(&hash)
        .fetch_one(&pool)
        .await
        .expect("find owner subject")
    };
    let node_name = format!("node-force-{}", uuid::Uuid::new_v4().simple());
    let node_id = common::seed_runtime_node(&pool, &owner_email, &node_name).await;
    let agent_id = common::seed_node_agent(&pool, &ctx.domain_id, &node_id).await;

    // Insert a nodetoken so we can verify it is revoked.
    let jti = format!("jti-{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO nodetoken (jti, node_id, revoked, issued_at, expires_at) \
         VALUES ($1, $2, false, now(), now() + interval '1 day')",
    )
    .bind(&jti)
    .bind(&node_id)
    .execute(&pool)
    .await
    .expect("insert nodetoken");

    let s = server(pool.clone());
    let res = s
        .delete(&format!("/api/runtime/nodes/{node_id}?force=true"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert!(
        res.status_code().is_success(),
        "force-delete must succeed: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    assert_eq!(body["deleted"], true);
    assert_eq!(body["detached_agents"], 1);

    // Verify: runtimenode row is gone.
    let node_gone: Option<String> =
        sqlx::query_scalar("SELECT id FROM runtimenode WHERE id = $1")
            .bind(&node_id)
            .fetch_optional(&pool)
            .await
            .expect("check runtimenode");
    assert!(node_gone.is_none(), "runtimenode row must be deleted");

    // Verify: meshagent is detached (runtime_node_id = NULL, status = 'offline').
    let agent_row = sqlx::query(
        "SELECT runtime_node_id, status FROM meshagent WHERE id = $1",
    )
    .bind(&agent_id)
    .fetch_optional(&pool)
    .await
    .expect("check meshagent");
    let agent_row = agent_row.expect("meshagent row must still exist (identity preserved)");
    let rnid: Option<String> = agent_row.try_get("runtime_node_id").ok().flatten();
    assert!(rnid.is_none(), "meshagent.runtime_node_id must be NULL after detach");
    let status: String = agent_row.try_get("status").expect("status");
    assert_eq!(status, "offline", "meshagent.status must be 'offline' after detach");

    // Verify: nodetoken row is cascaded/gone (ON DELETE CASCADE on runtimenode).
    let token_gone: Option<String> =
        sqlx::query_scalar("SELECT jti FROM nodetoken WHERE jti = $1")
            .bind(&jti)
            .fetch_optional(&pool)
            .await
            .expect("check nodetoken");
    assert!(token_gone.is_none(), "nodetoken must be CASCADE-deleted with the node");
}

/// DELETE with no assigned agents (no force required) must succeed.
#[tokio::test]
async fn delete_node_success_no_agents() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let owner_email = {
        use edgeplane_tower::auth::hash_token;
        let hash = hash_token(&ctx.owner_session_token);
        sqlx::query_scalar::<_, String>(
            "SELECT subject FROM usersession WHERE token_hash = $1",
        )
        .bind(&hash)
        .fetch_one(&pool)
        .await
        .expect("find owner subject")
    };
    let node_name = format!("node-noagents-{}", uuid::Uuid::new_v4().simple());
    let node_id = common::seed_runtime_node(&pool, &owner_email, &node_name).await;
    let s = server(pool.clone());

    let res = s
        .delete(&format!("/api/runtime/nodes/{node_id}"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert!(
        res.status_code().is_success(),
        "no-agent delete must succeed: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    assert_eq!(body["deleted"], true);
    assert_eq!(body["detached_agents"], 0);

    let node_gone: Option<String> =
        sqlx::query_scalar("SELECT id FROM runtimenode WHERE id = $1")
            .bind(&node_id)
            .fetch_optional(&pool)
            .await
            .expect("check runtimenode after no-agent delete");
    assert!(node_gone.is_none(), "node must be deleted");
}

/// A node's own JWT must NOT be able to delete itself (FIX M1: node-self deleted
/// from authz).  Obtaining a real node JWT requires going through the full
/// registration flow so the token is signed with the server's actual ephemeral
/// key and inserted into the nodetoken revocation table.
///
/// Steps:
///   1. Create a join token (owner session).
///   2. Register a node with that join token — response includes `node_jwt`.
///   3. DELETE /runtime/nodes/{id} with the node's own JWT → expect 403.
#[tokio::test]
async fn delete_node_403_for_node_self_jwt() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // 1. Create a join token (expires in 300s is plenty for a test).
    let jt_res = s
        .post("/api/runtime/join-tokens")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"expires_in_seconds": 300}))
        .await;
    assert_eq!(
        jt_res.status_code(),
        201,
        "join token create failed: {}",
        jt_res.text()
    );
    let jt_body: serde_json::Value = jt_res.json();
    let bootstrap_token = jt_body["token"].as_str().expect("join token must contain 'token'");

    // 2. Register a node using the bootstrap token.
    let node_name = format!("node-selfdelete-{}", uuid::Uuid::new_v4().simple());
    let reg_res = s
        .post("/api/runtime/nodes/register")
        .json(&serde_json::json!({
            "node_name": node_name,
            "hostname": "test-host",
            "bootstrap_token": bootstrap_token,
        }))
        .await;
    assert_eq!(
        reg_res.status_code(),
        201,
        "node register failed: {}",
        reg_res.text()
    );
    let reg_body: serde_json::Value = reg_res.json();
    let node_id = reg_body["id"].as_str().expect("register must return id");
    let node_jwt = reg_body["node_jwt"].as_str().expect("register must return node_jwt");

    // 3. Attempt self-delete with the node's own JWT — must be 403.
    //    Per spec §M1: node-self is excluded from delete authz; only owner
    //    session or admin may perform an irreversible DELETE.
    let del_res = s
        .delete(&format!("/api/runtime/nodes/{node_id}"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {node_jwt}"),
        )
        .await;
    assert_eq!(
        del_res.status_code(),
        403,
        "node self-delete via node JWT must be forbidden (M1): {}",
        del_res.text()
    );

    // Confirm the node was NOT deleted (the 403 must have been a hard stop).
    let still_exists: Option<String> = sqlx::query_scalar(
        "SELECT id FROM runtimenode WHERE id = $1",
    )
    .bind(node_id)
    .fetch_optional(&pool)
    .await
    .expect("check runtimenode after self-delete attempt");
    assert!(
        still_exists.is_some(),
        "node must still exist after a forbidden self-delete attempt"
    );
}
