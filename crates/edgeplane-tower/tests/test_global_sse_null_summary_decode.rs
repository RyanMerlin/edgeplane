//! Regression test: `global_sse`'s background poll loop (`routes/work.rs`,
//! `GET /sse`) must not panic decoding `meshprogressevent.summary` when it is
//! NULL. `summary` is nullable `text`
//! (`crates/edgeplane-tower/migrations/0001_initial_schema.sql`); the MCP
//! `progress_mesh_task` handler's INSERT (`routes/mcp.rs`) never includes a
//! `summary` column at all, so every mesh-task progress event submitted
//! through the MCP tool — the path the fleet's own `edgeplane task mesh
//! progress` / `progress_mesh_task` calls use — lands with `summary = NULL`.
//! Decoding that as non-Option `String` panics via `Row::get`'s internal
//! `try_get().unwrap()`, same failure mechanism as the meshmessage/
//! meshprogressevent bugs already fixed in #113.
//!
//! `global_sse` runs this decode inside a detached `tokio::spawn`'d polling
//! loop (2s interval, admin-gated, never terminates on its own). A correct
//! SSE stream has no end, so collecting a full HTTP response body (the only
//! mode `axum-test` supports — no partial-read/timeout API) would hang
//! forever on the fixed/working code path; there is no safe way to drive
//! `global_sse` itself end-to-end in a bounded test. Instead this test
//! reproduces the exact query the loop issues against a real Postgres row
//! seeded with a NULL summary (the same shape `progress_mesh_task` produces)
//! and asserts the fixed `Option<String>` decode succeeds.
//!
//! Caveat: because this re-implements the decode rather than calling
//! `global_sse` directly (it isn't `pub` and can't be), it does NOT fail if
//! the fix in `routes/work.rs` is reverted — it only proves the decode this
//! fix relies on is sound against a real row of this shape. Confirmed by
//! testing: reverting the `work.rs` fix leaves this test green.

mod common;

use common::{seed_ready_task, setup};
use sqlx::Row;

#[tokio::test]
async fn meshprogressevent_summary_decodes_when_null() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id = seed_ready_task(&pool, &ctx.mission_id, &ctx.domain_id).await;

    // Mirrors the MCP progress_mesh_task INSERT (routes/mcp.rs), which omits
    // `summary` entirely — the column has no DB default, so it lands NULL.
    sqlx::query(
        "INSERT INTO meshprogressevent (task_id, agent_id, seq, event_type, phase, step, payload_json, occurred_at) \
         VALUES ($1, 'harness', 0, 'phase_finished', NULL, NULL, '{}', now())",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .expect("insert meshprogressevent with NULL summary");

    // Exact query global_sse's poll loop issues.
    let rows = sqlx::query(
        "SELECT id, task_id, agent_id, seq, event_type, phase, step, summary, occurred_at \
         FROM meshprogressevent WHERE id > $1 ORDER BY id ASC LIMIT 100",
    )
    .bind(0i32)
    .fetch_all(&pool)
    .await
    .expect("fetch meshprogressevent rows");

    let row = rows
        .iter()
        .find(|r| r.get::<String, _>("task_id") == task_id)
        .expect("seeded row must be present");

    // This is the exact decode global_sse now uses. Before the fix
    // (`row.get::<String, _>("summary")`), this line would panic.
    let summary: Option<String> = row.get("summary");
    assert_eq!(summary, None, "NULL summary must decode to None, not panic");
}
