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
        .get(&format!("/missions/{mission_id}/tasks?status=ready"))
        .await
        .unwrap_or_default();

    // Also fetch broadcast tasks that are already running so every agent joins.
    let broadcast_running: Vec<MeshTaskRecord> = client
        .get(&format!("/missions/{mission_id}/tasks?status=running"))
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
        .post_empty(&format!("/tasks/{task_id}/claim"))
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
        .raw_post_no_throw(&format!("/tasks/{task_id}/heartbeat"), &body)
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
        .raw_post(&format!("/tasks/{task_id}/progress"), &body)
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
        .raw_post_no_throw(&format!("/tasks/{task_id}/complete"), &body)
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
            .get::<Vec<serde_json::Value>>(&format!("/tasks/{dep_id}/progress?since_seq=0"))
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
        .raw_post_no_throw(&format!("/tasks/{task_id}/fail"), &body)
        .await
        .map_err(TaskError::Other)?;

    if resp.status() == reqwest::StatusCode::CONFLICT {
        return Err(TaskError::LeaseMismatch);
    }
    resp.error_for_status().map_err(|e| TaskError::Other(anyhow!(e)))?;
    Ok(())
}
