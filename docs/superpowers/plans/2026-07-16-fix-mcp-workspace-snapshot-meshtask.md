> **Historical record.** The `task`/`meshtask` split this plan diagnoses and works around was resolved
> by migration `0014_unify_task_meshtask.sql` (2026-07-26), which merged both tables into one `public.task`
> table with a `kind` discriminator. This plan is preserved as-is for the record; do not use it as current
> architecture guidance. See `docs/architecture/entities.md` § Task.

# Fix MCP Workspace Snapshot to Read meshtask Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix `build_workspace_snapshot` (backing the `load_mission_workspace` MCP tool) so it reflects the live, agent-claimable `meshtask` table instead of the disconnected, UI-only `task` table — today an agent can claim/complete real work via `work.rs`/MCP claim tools and `load_mission_workspace` will never show it (or show it as absent), because the two tables have zero synchronization.

**Architecture:** No design change, no new table, no sync mechanism added — `task` and `meshtask` are legitimately two separate tables owned by two separate subsystems (`task` = legacy UI/CRUD surface owned by `tasks.rs`/`explorer.rs`/`search.rs`; `meshtask` = agent dispatch surface owned by `work.rs` and the MCP claim/heartbeat/complete tools). The fix is to point the one function that's supposed to give agents visibility into *their own claimable work* (`build_workspace_snapshot`, called only from the `load_mission_workspace`/`commit_mission_workspace` MCP tools) at the table that subsystem actually operates on.

**Tech Stack:** Rust (edition 2024), `sqlx` (raw queries, no compile-time macros — matches existing file convention), `axum-test` for HTTP-level integration tests (already used throughout `edgeplane-tower/tests/`).

## Global Constraints

- Do not modify the `task` table, `tasks.rs`, `explorer.rs`, `search.rs`, or `missions.rs` — they are a separate, intentionally-legacy subsystem, out of scope.
- Do not add any sync/mirroring mechanism between `task` and `meshtask` — the fix is to stop reading the wrong table, not to unify the two.
- `meshtask` has no `public_id`, `owner`, or `labels` columns — do not invent values for these; drop them from the snapshot's task JSON shape. Confirmed no consumer depends on them: `crates/edgeplane/src/commands.rs`'s `extract_workspace_state` only reads `lease`-level fields (`id`, `domain_id`, `mission_id`, `status`), never `workspace_snapshot.tasks[].*`.
- Filter semantics must mirror the original query's intent (exclude terminal-state tasks) using `meshtask`'s real status vocabulary (`pending`, `ready`, `claimed`, `running`, `blocked`, `waiting_review`, `finished`, `failed`, `cancelled` — confirmed via `grep` across `work.rs`), not invented values.

---

### Task 1: Point `build_workspace_snapshot`'s task query at `meshtask`, with regression tests proving the fix

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/mcp.rs:1938-2002` (the `build_workspace_snapshot` function)
- Create: `crates/edgeplane-tower/tests/test_mcp_workspace_snapshot.rs`

**Interfaces:**
- Consumes: `crates/edgeplane-tower/tests/common/mod.rs`'s `setup() -> Option<(PgPool, Ctx)>` (env-gated on `TEST_DATABASE_URL`; skips cleanly if unset, matching every other DB-backed test in this crate) and `seed_ready_task(db, mission_id, domain_id) -> String` (already exists, inserts a `meshtask` row with `status='ready'`).
- Produces: `build_workspace_snapshot`'s signature (`async fn build_workspace_snapshot(db: &sqlx::PgPool, domain_id: &str, mission_id: &str) -> Value`) is unchanged — both call sites (`mcp.rs:1453` inside `load_mission_workspace`, `mcp.rs:1816` inside `commit_mission_workspace`) need no changes. Only the returned JSON's `"tasks"` array element shape changes: was `{id: i32, public_id, title, description, status, owner, updated_at}`, becomes `{id: String, title, description, status, priority: i32, claimed_by_agent_id, claim_policy, updated_at}`.

- [ ] **Step 1: Write the failing tests**

Create `crates/edgeplane-tower/tests/test_mcp_workspace_snapshot.rs`:

```rust
//! Regression tests: `load_mission_workspace`'s task snapshot must reflect
//! the live, agent-claimable `meshtask` table, not the disconnected legacy
//! `task` table (the two have zero synchronization — see
//! docs/superpowers/plans/2026-07-16-fix-mcp-workspace-snapshot-meshtask.md).

mod common;

use axum_test::TestServer;
use common::{seed_ready_task, setup};
use edgeplane_tower::{AppConfig, build_app};
use sqlx::PgPool;
use uuid::Uuid;

fn server(pool: PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

/// Insert a meshtask row with an explicit status (any value, including
/// terminal states not covered by the shared `common::seed_*` helpers).
async fn seed_task_with_status(db: &PgPool, mission_id: &str, domain_id: &str, status: &str) -> String {
    let task_id = format!("task-{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO meshtask \
         (id, mission_id, domain_id, title, description, input_json, claim_policy, \
          depends_on, produces, consumes, required_capabilities, \
          status, priority, version_counter, created_by_subject, \
          created_at, updated_at) \
         VALUES ($1, $2, $3, 'test-task', '', '{}', 'any', '[]', '{}', '{}', '[]', \
                 $4, 0, 1, 'harness', now(), now())",
    )
    .bind(&task_id)
    .bind(mission_id)
    .bind(domain_id)
    .bind(status)
    .execute(db)
    .await
    .expect("insert meshtask with status");
    task_id
}

#[tokio::test]
async fn load_mission_workspace_reflects_meshtask_not_legacy_task() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id = seed_ready_task(&pool, &ctx.mission_id, &ctx.domain_id).await;

    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "load_mission_workspace",
            "args": { "mission_id": ctx.mission_id }
        }))
        .await;

    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    assert_eq!(body["ok"], true, "response body: {body}");

    let tasks = body["result"]["workspace_snapshot"]["tasks"]
        .as_array()
        .expect("tasks must be an array");
    assert_eq!(
        tasks.len(),
        1,
        "expected exactly the one seeded meshtask; snapshot must read from \
         meshtask, not the disconnected task table: {body}"
    );
    assert_eq!(tasks[0]["id"], task_id);
    assert_eq!(tasks[0]["status"], "ready");
}

#[tokio::test]
async fn load_mission_workspace_excludes_terminal_meshtasks() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let ready_id = seed_ready_task(&pool, &ctx.mission_id, &ctx.domain_id).await;
    let _finished_id = seed_task_with_status(&pool, &ctx.mission_id, &ctx.domain_id, "finished").await;
    let _cancelled_id = seed_task_with_status(&pool, &ctx.mission_id, &ctx.domain_id, "cancelled").await;

    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "load_mission_workspace",
            "args": { "mission_id": ctx.mission_id }
        }))
        .await;

    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    let tasks = body["result"]["workspace_snapshot"]["tasks"]
        .as_array()
        .expect("tasks must be an array");
    assert_eq!(
        tasks.len(),
        1,
        "only the non-terminal (ready) task should appear: {body}"
    );
    assert_eq!(tasks[0]["id"], ready_id);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `TEST_DATABASE_URL=<your test postgres url> cargo test -p edgeplane-tower --test test_mcp_workspace_snapshot -- --nocapture`

Expected: FAIL. Both tests should fail on `assert_eq!(tasks.len(), 1, ...)` with `tasks.len() == 0` — today `build_workspace_snapshot` queries the empty `task` table (nothing was seeded there), so the snapshot's `tasks` array is empty regardless of what's in `meshtask`. If `TEST_DATABASE_URL` is unset, both tests silently return early (`setup()` returns `None`) — set it before running this step, or run against whatever ephemeral Postgres this workspace's CI/other DB-backed tests already use (check `crates/edgeplane-tower/tests/test_authz.rs`'s own run instructions / CI config for the connection string convention this repo uses).

- [ ] **Step 3: Rewrite `build_workspace_snapshot` to query `meshtask`**

In `crates/edgeplane-tower/src/routes/mcp.rs`, replace the entire function (lines 1938-2002) with:

```rust
async fn build_workspace_snapshot(db: &sqlx::PgPool, domain_id: &str, mission_id: &str) -> Value {
    let tasks = sqlx::query(
        "SELECT id, title, description, status, priority, claimed_by_agent_id, claim_policy, updated_at \
         FROM meshtask WHERE mission_id=$1 AND status NOT IN ('finished','cancelled') ORDER BY updated_at DESC LIMIT 200"
    )
    .bind(mission_id).fetch_all(db).await.unwrap_or_default();

    let docs = sqlx::query(
        "SELECT id, title, doc_type, status, version, updated_at FROM doc WHERE mission_id=$1 ORDER BY updated_at DESC LIMIT 100"
    )
    .bind(mission_id).fetch_all(db).await.unwrap_or_default();

    let artifacts = sqlx::query(
        "SELECT id, name, artifact_type, uri, storage_backend, mime_type, size_bytes, status, version, updated_at \
         FROM artifact WHERE mission_id=$1 ORDER BY updated_at DESC LIMIT 100"
    )
    .bind(mission_id).fetch_all(db).await.unwrap_or_default();

    // Build version index for conflict detection (stored in base_snapshot_json)
    let mut index = serde_json::Map::new();
    for r in &docs {
        let id: i32 = r.get("id");
        let ver: i32 = r.try_get("version").unwrap_or(1);
        index.insert(format!("doc:{id}"), json!(ver));
    }
    for r in &artifacts {
        let id: i32 = r.get("id");
        let ver: i32 = r.try_get("version").unwrap_or(1);
        index.insert(format!("artifact:{id}"), json!(ver));
    }

    json!({
        "domain_id": domain_id,
        "mission_id": mission_id,
        "tasks": tasks.iter().map(|r| json!({
            "id": r.get::<String,_>("id"),
            "title": r.get::<String,_>("title"),
            "description": r.try_get::<String,_>("description").unwrap_or_default(),
            "status": r.get::<String,_>("status"),
            "priority": r.get::<i32,_>("priority"),
            "claimed_by_agent_id": r.try_get::<String,_>("claimed_by_agent_id").unwrap_or_default(),
            "claim_policy": r.get::<String,_>("claim_policy"),
            "updated_at": r.get::<chrono::NaiveDateTime,_>("updated_at"),
        })).collect::<Vec<_>>(),
        "docs": docs.iter().map(|r| json!({
            "id": r.get::<i32,_>("id"),
            "title": r.get::<String,_>("title"),
            "doc_type": r.get::<String,_>("doc_type"),
            "status": r.get::<String,_>("status"),
            "version": r.try_get::<i32,_>("version").unwrap_or(1),
            "updated_at": r.get::<chrono::NaiveDateTime,_>("updated_at"),
        })).collect::<Vec<_>>(),
        "artifacts": artifacts.iter().map(|r| json!({
            "id": r.get::<i32,_>("id"),
            "name": r.get::<String,_>("name"),
            "artifact_type": r.try_get::<String,_>("artifact_type").unwrap_or_default(),
            "storage_backend": r.try_get::<String,_>("storage_backend").unwrap_or_default(),
            "mime_type": r.try_get::<String,_>("mime_type").unwrap_or_default(),
            "size_bytes": r.try_get::<i32,_>("size_bytes").unwrap_or(0),
            "status": r.try_get::<String,_>("status").unwrap_or_default(),
            "version": r.try_get::<i32,_>("version").unwrap_or(1),
            "updated_at": r.get::<chrono::NaiveDateTime,_>("updated_at"),
        })).collect::<Vec<_>>(),
        "index": index,
    })
}
```

Only the `tasks` query and its JSON-mapping closure changed; `docs`, `artifacts`, and `index` are verbatim from the original — confirm the diff is scoped to just those two blocks when reviewing.

- [ ] **Step 4: Run tests to verify they pass**

Run: `TEST_DATABASE_URL=<your test postgres url> cargo test -p edgeplane-tower --test test_mcp_workspace_snapshot -- --nocapture`

Expected: PASS — both tests green.

- [ ] **Step 5: Run the full existing MCP + work test suites to check for regressions**

Run: `TEST_DATABASE_URL=<your test postgres url> cargo test -p edgeplane-tower --test mcp_parity --test test_work --test test_authz -- --nocapture`

Expected: PASS — `mcp_parity` only checks tool-catalogue/dispatch-arm parity (unaffected by this change), `test_work`/`test_authz` don't touch `build_workspace_snapshot`. This step exists to catch anything unexpected, not because a regression is anticipated.

- [ ] **Step 6: Full workspace check**

Run: `cargo check --workspace && cargo clippy -p edgeplane-tower --all-targets -- -D warnings`

Expected: clean, no errors, no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/edgeplane-tower/src/routes/mcp.rs \
        crates/edgeplane-tower/tests/test_mcp_workspace_snapshot.rs
git commit -m "fix(tower): load_mission_workspace snapshot reads meshtask, not legacy task

build_workspace_snapshot (backing load_mission_workspace / commit_mission_workspace)
was querying the task table -- a separate, UI-only surface owned by tasks.rs/
explorer.rs/search.rs with zero synchronization to meshtask, the table every
real agent-dispatch operation (work.rs claim/heartbeat/complete, MCP claim
tools) actually reads and writes. An agent could claim and complete real work
that this snapshot never reflected. Points the query at meshtask and maps its
real columns (drops public_id/owner/labels, which don't exist there; adds
priority/claimed_by_agent_id/claim_policy).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```
