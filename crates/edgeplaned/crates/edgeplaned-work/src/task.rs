use edgeplaned_core::client::BackendClient;
use edgeplaned_core::types::{DependencyResult, MeshTaskRecord};
use anyhow::{anyhow, Result};

/// Result of a successful task claim.
pub struct ClaimResult {
    pub task: MeshTaskRecord,
    pub claim_lease_id: Option<String>,
}

/// Error type that distinguishes a 409 lease-mismatch response from other errors.
#[derive(Debug)]
pub enum TaskError {
    LeaseMismatch,
    Other(anyhow::Error),
}

impl std::fmt::Display for TaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskError::LeaseMismatch => write!(f, "lease mismatch (409)"),
            TaskError::Other(e) => write!(f, "{e}"),
        }
    }
}

impl From<anyhow::Error> for TaskError {
    fn from(e: anyhow::Error) -> Self {
        TaskError::Other(e)
    }
}

/// Poll the backend for tasks this agent can work on.
///
/// Returns `ready` tasks (all policies) plus `running` broadcast tasks — an
/// agent can join a broadcast task that's already been claimed by other agents.
pub async fn poll_ready_tasks(
    client: &BackendClient,
    mission_id: &str,
    _capabilities: &[edgeplaned_core::types::Capability],
) -> Result<Vec<MeshTaskRecord>> {
    let mut ready: Vec<MeshTaskRecord> = client
        .get(&format!("/work/missions/{mission_id}/tasks?status=ready"))
        .await
        .unwrap_or_default();

    // Also fetch broadcast tasks that are already running so every agent joins.
    let broadcast_running: Vec<MeshTaskRecord> = client
        .get(&format!("/work/missions/{mission_id}/tasks?status=running"))
        .await
        .unwrap_or_default();

    for t in broadcast_running {
        if t.claim_policy == "broadcast" {
            ready.push(t);
        }
    }

    Ok(ready)
}

/// Claim a task.  Returns a `ClaimResult` containing the task record (with
/// status set to "claimed") and the `claim_lease_id` returned by the backend.
pub async fn claim_task(client: &BackendClient, task_id: &str) -> Result<ClaimResult> {
    let resp: serde_json::Value = client
        .post_empty(&format!("/work/tasks/{task_id}/claim"))
        .await?;

    let claim_lease_id = resp
        .get("claim_lease_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    // The claim endpoint returns the full task record; deserialise it if
    // possible, otherwise fall back to a minimal record.
    let task: MeshTaskRecord = serde_json::from_value(resp).map_err(|e| anyhow!(e))?;

    Ok(ClaimResult { task, claim_lease_id })
}

/// Send a heartbeat to renew the lease on a running task.
///
/// Returns `Err(TaskError::LeaseMismatch)` when the backend responds 409.
pub async fn heartbeat_task(
    client: &BackendClient,
    task_id: &str,
    claim_lease_id: Option<&str>,
) -> Result<(), TaskError> {
    let mut body = serde_json::json!({});
    if let Some(lid) = claim_lease_id {
        body["claim_lease_id"] = serde_json::Value::String(lid.to_string());
    }
    let resp = client
        .raw_post_no_throw(&format!("/work/tasks/{task_id}/heartbeat"), &body)
        .await
        .map_err(TaskError::Other)?;

    if resp.status() == reqwest::StatusCode::CONFLICT {
        return Err(TaskError::LeaseMismatch);
    }
    resp.error_for_status().map_err(|e| TaskError::Other(anyhow!(e)))?;
    Ok(())
}

/// Post a typed progress event.
pub async fn post_progress(
    client: &BackendClient,
    task_id: &str,
    event: &edgeplaned_core::progress::ProgressEvent,
) -> Result<()> {
    use serde_json::json;
    let body = json!({
        "event_type": event.event_type.to_string(),
        "phase": event.phase,
        "step": event.step,
        "summary": event.summary,
        "payload_json": event.payload.to_string(),
    });
    client
        .raw_post(&format!("/work/tasks/{task_id}/progress"), &body)
        .await?;
    Ok(())
}

/// Mark a task complete.
///
/// Returns `Err(TaskError::LeaseMismatch)` when the backend responds 409.
pub async fn complete_task(
    client: &BackendClient,
    task_id: &str,
    claim_lease_id: Option<&str>,
    result_artifact_id: Option<&str>,
) -> Result<(), TaskError> {
    let mut body = serde_json::json!({ "result_artifact_id": result_artifact_id });
    if let Some(lid) = claim_lease_id {
        body["claim_lease_id"] = serde_json::Value::String(lid.to_string());
    }
    let resp = client
        .raw_post_no_throw(&format!("/work/tasks/{task_id}/complete"), &body)
        .await
        .map_err(TaskError::Other)?;

    if resp.status() == reqwest::StatusCode::CONFLICT {
        return Err(TaskError::LeaseMismatch);
    }
    resp.error_for_status().map_err(|e| TaskError::Other(anyhow!(e)))?;
    Ok(())
}

/// Fetch the most recent `phase_finished` summary for each upstream
/// dependency, ready to inline as `[DEPENDENCY RESULTS]` in the downstream
/// task's prompt.
///
/// Best-effort: per-dep failures are logged and skipped — a missing
/// dependency result must never block injection of the downstream task.
pub async fn fetch_dependency_results(
    client: &BackendClient,
    depends_on: &[String],
) -> Vec<DependencyResult> {
    let mut out = Vec::with_capacity(depends_on.len());
    for dep_id in depends_on {
        match client
            .get::<Vec<serde_json::Value>>(&format!("/work/tasks/{dep_id}/progress?since_seq=0"))
            .await
        {
            Ok(events) => {
                let last_phase_finished = events.iter().rev().find(|e| {
                    e.get("event_type").and_then(|v| v.as_str()) == Some("phase_finished")
                });
                let Some(ev) = last_phase_finished else {
                    tracing::debug!(
                        "fetch_dependency_results: dep {dep_id} has no phase_finished event yet"
                    );
                    continue;
                };
                let summary = ev
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let finished_at = ev
                    .get("occurred_at")
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default();
                out.push(DependencyResult {
                    task_id: dep_id.clone(),
                    title: dep_id.clone(),
                    summary,
                    finished_at,
                });
            }
            Err(e) => {
                tracing::debug!("fetch_dependency_results: dep {dep_id}: {e}");
            }
        }
    }
    out
}

/// Mark a task failed.
///
/// Returns `Err(TaskError::LeaseMismatch)` when the backend responds 409.
pub async fn fail_task(
    client: &BackendClient,
    task_id: &str,
    claim_lease_id: Option<&str>,
    error: &str,
) -> Result<(), TaskError> {
    let mut body = serde_json::json!({ "error": error });
    if let Some(lid) = claim_lease_id {
        body["claim_lease_id"] = serde_json::Value::String(lid.to_string());
    }
    let resp = client
        .raw_post_no_throw(&format!("/work/tasks/{task_id}/fail"), &body)
        .await
        .map_err(TaskError::Other)?;

    if resp.status() == reqwest::StatusCode::CONFLICT {
        return Err(TaskError::LeaseMismatch);
    }
    resp.error_for_status().map_err(|e| TaskError::Other(anyhow!(e)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Every field of MeshTaskRecord, explicit, so JSON deserialization can't
    /// silently rely on serde defaults we haven't verified.
    fn mesh_task_json(id: &str) -> serde_json::Value {
        serde_json::json!({
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
            payload: serde_json::json!({}),
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
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![serde_json::json!({
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
