> **Historical record.** The `task`/`meshtask` split referenced throughout this plan was resolved by
> migration `0014_unify_task_meshtask.sql` (2026-07-26), which merged both tables into one `public.task`
> table with a `kind` discriminator. This plan is preserved as-is for the record; do not use it as current
> architecture guidance. See `docs/architecture/entities.md` § Task.

# Fix edgeplaned-work Task-Claim Client Path Prefix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix `edgeplaned-work`'s `task.rs` HTTP client so `SessionMode::Task` agents (the daemon's `#[default]` session mode) can actually claim, heartbeat, progress, complete, and fail MeshTasks against the live `edgeplane-tower` binary — today every one of these calls 404s because the client omits the `/work` path segment that `edgeplane-tower/src/routes/work.rs`'s router requires.

**Architecture:** No design change. `edgeplaned-work/src/task.rs` builds request paths as bare strings inline in each function (e.g. `format!("/tasks/{task_id}/claim")`); `edgeplaned-bin/src/daemon.rs` constructs the `BackendClient` with `api_prefix = "/api"` (env-overridable via `EP_API_PREFIX`), so the client's actual outgoing request is `<base>/api/tasks/{task_id}/claim`. The tower's `work::router()` registers these same operations at `/work/tasks/{task_id}/claim` etc., merged flat under the tower's own `/api` nest (`edgeplane-tower/src/routes/mod.rs`), so the real, currently-served route is `/api/work/tasks/{task_id}/claim`. The fix is a pure string change: add the missing `/work` segment to every path literal in `task.rs`. No route, schema, or protocol change on the tower side — `work.rs`'s routes are current and were actively touched 2 days ago in the authz-hardening work (`#106`, `#97`); they are the side that's correct.

**Tech Stack:** Rust (edition 2024), `reqwest` (via `edgeplaned-core::client::BackendClient`), `wiremock = "0.6"` for HTTP-level regression tests (already a blessed workspace dependency — used in `edgeplane` and `edgeplane-tower`, net-new only to `edgeplaned-work`).

## Global Constraints

- Do not change any route in `edgeplane-tower/src/routes/work.rs` — those routes are current and correct; this plan only changes the client.
- Do not change `edgeplaned-bin/src/task_worker.rs` — it already uses the correct `/work/tasks/{task_id}/...` paths and is out of scope.
- Do not change `BackendClient::api_prefix` default or `daemon.rs`'s `/api` prefix wiring — those are unrelated and correct.
- Every path literal changed must be verified against the exact route strings in `crates/edgeplane-tower/src/routes/work.rs:77-120` (`router()`), not assumed.

---

### Task 1: Add `/work` prefix to every `task.rs` request path, with regression tests proving each one

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-work/Cargo.toml`
- Modify: `crates/edgeplaned/crates/edgeplaned-work/src/task.rs`
- Modify: `crates/edgeplaned/crates/edgeplaned-core/src/client.rs:6-14` (stale doc comment)

**Interfaces:**
- Consumes: `edgeplaned_core::client::BackendClient` (unchanged public API — `new(base_url, token) -> Self`, `get<T>(path)`, `post_empty<T>(path)`, `raw_post_no_throw(path, body)`).
- Produces: no signature changes. `poll_ready_tasks`, `claim_task`, `heartbeat_task`, `post_progress`, `complete_task`, `fetch_dependency_results`, `fail_task` keep their exact current signatures — only the internal path strings change. `task_loop.rs`/`claim.rs` (the only current callers) need no changes.

- [ ] **Step 1: Add `wiremock` as a dev-dependency**

Edit `crates/edgeplaned/crates/edgeplaned-work/Cargo.toml` — it currently has no `[dev-dependencies]` section. Add one at the end of the file:

```toml
[dev-dependencies]
wiremock = "0.6"
```

- [ ] **Step 2: Write the failing tests**

Add this `#[cfg(test)] mod tests` block at the end of `crates/edgeplaned/crates/edgeplaned-work/src/task.rs` (after the existing `fail_task` function, i.e. append after line 227):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Every field of MeshTaskRecord, explicit, so JSON deserialization can't
    /// silently rely on serde defaults we haven't verified.
    fn mesh_task_json(id: &str) -> serde_json::Value {
        json!({
            "id": id,
            "mission_id": "m-1",
            "domain_id": "d-1",
            "title": "test",
            "description": "",
            "status": "claimed",
            "claim_policy": "exclusive",
            "required_capabilities": [],
            "lease_expires_at": null,
            "claim_lease_id": "lease-abc",
            "depends_on": [],
            "produces": {},
            "consumes": {}
        })
    }

    #[tokio::test]
    async fn poll_ready_tasks_hits_work_prefixed_paths() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/work/missions/m-1/tasks"))
            .and(query_param("status", "ready"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![mesh_task_json("t-1")]))
            .expect(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/work/missions/m-1/tasks"))
            .and(query_param("status", "running"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<serde_json::Value>::new()))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = BackendClient::new(mock_server.uri(), "test-token");
        let result = poll_ready_tasks(&client, "m-1", &[]).await;

        assert!(result.is_ok(), "poll_ready_tasks should succeed: {:?}", result.err());
        assert_eq!(result.unwrap().len(), 1, "should return the one ready task from the mock");
        mock_server.verify().await;
    }

    #[tokio::test]
    async fn claim_task_hits_work_prefixed_path() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/work/tasks/t-1/claim"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mesh_task_json("t-1")))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = BackendClient::new(mock_server.uri(), "test-token");
        let result = claim_task(&client, "t-1").await;

        assert!(result.is_ok(), "claim_task should succeed against /work/tasks/{{id}}/claim: {:?}", result.err());
        assert_eq!(result.unwrap().claim_lease_id.as_deref(), Some("lease-abc"));
    }

    #[tokio::test]
    async fn heartbeat_task_hits_work_prefixed_path() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/work/tasks/t-1/heartbeat"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = BackendClient::new(mock_server.uri(), "test-token");
        let result = heartbeat_task(&client, "t-1", Some("lease-abc")).await;

        assert!(result.is_ok(), "heartbeat_task should succeed against /work/tasks/{{id}}/heartbeat: {:?}", result.err());
    }

    #[tokio::test]
    async fn post_progress_hits_work_prefixed_path() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/work/tasks/t-1/progress"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = BackendClient::new(mock_server.uri(), "test-token");
        let event = edgeplaned_core::progress::ProgressEvent {
            event_type: edgeplaned_core::progress::ProgressEventType::PhaseStarted,
            phase: Some("test-phase".to_string()),
            step: None,
            summary: "test".to_string(),
            payload: json!({}),
        };
        let result = post_progress(&client, "t-1", &event).await;

        assert!(result.is_ok(), "post_progress should succeed against /work/tasks/{{id}}/progress: {:?}", result.err());
    }

    #[tokio::test]
    async fn complete_task_hits_work_prefixed_path() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/work/tasks/t-1/complete"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = BackendClient::new(mock_server.uri(), "test-token");
        let result = complete_task(&client, "t-1", Some("lease-abc"), None).await;

        assert!(result.is_ok(), "complete_task should succeed against /work/tasks/{{id}}/complete: {:?}", result.err());
    }

    #[tokio::test]
    async fn fail_task_hits_work_prefixed_path() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/work/tasks/t-1/fail"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = BackendClient::new(mock_server.uri(), "test-token");
        let result = fail_task(&client, "t-1", Some("lease-abc"), "test error").await;

        assert!(result.is_ok(), "fail_task should succeed against /work/tasks/{{id}}/fail: {:?}", result.err());
    }

    #[tokio::test]
    async fn fetch_dependency_results_hits_work_prefixed_path() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/work/tasks/dep-1/progress"))
            .and(query_param("since_seq", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![json!({
                "event_type": "phase_finished",
                "summary": "done",
                "occurred_at": "2026-07-16T00:00:00Z"
            })]))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = BackendClient::new(mock_server.uri(), "test-token");
        let results = fetch_dependency_results(&client, &["dep-1".to_string()]).await;

        assert_eq!(results.len(), 1, "should surface the one phase_finished event from the mock");
        assert_eq!(results[0].summary, "done");
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p edgeplaned-work --lib task::tests -- --nocapture`

Expected: FAIL. Every test should fail — either the mock's `.expect(1)` assertion panics (mock never hit because the client requested `/missions/...`/`/tasks/...` without `/work`), or the function-level `assert!(result.is_ok())` fails because the un-mocked path returned wiremock's default 404. If any test instead fails to *compile*, fix the compile error only (e.g. a field-name or import mismatch) — do not change the assertions — then re-run before proceeding.

- [ ] **Step 4: Fix the path strings in `task.rs`**

In `crates/edgeplaned/crates/edgeplaned-work/src/task.rs`, change exactly these 8 format strings (leave every other line untouched):

```rust
// poll_ready_tasks — both calls:
        .get(&format!("/work/missions/{mission_id}/tasks?status=ready"))
```
```rust
    let broadcast_running: Vec<MeshTaskRecord> = client
        .get(&format!("/work/missions/{mission_id}/tasks?status=running"))
```
```rust
// claim_task:
        .post_empty(&format!("/work/tasks/{task_id}/claim"))
```
```rust
// heartbeat_task:
        .raw_post_no_throw(&format!("/work/tasks/{task_id}/heartbeat"), &body)
```
```rust
// post_progress:
        .raw_post(&format!("/work/tasks/{task_id}/progress"), &body)
```
```rust
// complete_task:
        .raw_post_no_throw(&format!("/work/tasks/{task_id}/complete"), &body)
```
```rust
// fetch_dependency_results:
            .get::<Vec<serde_json::Value>>(&format!("/work/tasks/{dep_id}/progress?since_seq=0"))
```
```rust
// fail_task:
        .raw_post_no_throw(&format!("/work/tasks/{task_id}/fail"), &body)
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p edgeplaned-work --lib task::tests -- --nocapture`

Expected: PASS — all 7 tests green.

- [ ] **Step 6: Fix the stale doc comment on `BackendClient`**

The comment at `crates/edgeplaned/crates/edgeplaned-core/src/client.rs:6-14` claims `/work/` is deprecated legacy behavior and the controlplane serves routes "at the root" by default. That was true when written (2026-05-11, before `edgeplane-tower/src/routes/work.rs`'s dispatch API existed in its current form) but is now actively misleading — `work.rs`'s `/work`-prefixed routes are current, actively maintained (last touched 2026-07-14 in the authz-hardening work), and required for all MeshTask dispatch operations. `mint_agent_token` and `rotate_node_token` further down the same file already use bare paths correctly per their own current conventions.

Replace lines 6-14:

```rust
/// Thin HTTP client with bearer auth for the Edgeplane backend.
///
/// `api_prefix` is prepended to every path passed into `get`/`post`/etc. The
/// daemon sets it to `/api` (see `edgeplaned-bin/src/config.rs`, env-overridable
/// via `EP_API_PREFIX`) to match the tower's `.nest("/api", ...)` mount.
///
/// Within that `/api` root, MeshTask dispatch operations (claim, heartbeat,
/// progress, complete, fail, and mission task listing) live under a further
/// `/work` segment — see `edgeplane-tower/src/routes/work.rs`'s `router()`.
/// Callers of those operations must include the `/work` segment explicitly in
/// the path passed to `get`/`post`/etc.; it is not part of `api_prefix`.
```

- [ ] **Step 7: Full workspace check**

Run: `cargo check --workspace && cargo clippy -p edgeplaned-work -p edgeplaned-core --all-targets -- -D warnings`

Expected: clean, no errors, no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-work/Cargo.toml \
        crates/edgeplaned/crates/edgeplaned-work/src/task.rs \
        crates/edgeplaned/crates/edgeplaned-core/src/client.rs
git commit -m "fix(edgeplaned-work): add missing /work prefix to task-claim client paths

SessionMode::Task agents' claim/heartbeat/progress/complete/fail calls were
silently 404ing against the live tower — edgeplaned-work's task.rs never
picked up the /work prefix that edgeplane-tower/routes/work.rs's router
requires (task_worker.rs, a separate one-shot dispatch path, already used it
correctly). poll_ready_tasks swallowed the 404 via unwrap_or_default(), so
the symptom was an agent silently seeing zero ready tasks rather than a
visible error.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```
