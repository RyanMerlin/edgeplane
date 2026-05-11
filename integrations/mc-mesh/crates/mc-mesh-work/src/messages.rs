use mc_mesh_core::client::BackendClient;
use anyhow::Result;

/// Send a message scoped to a kluster.
pub async fn send_kluster_message(
    client: &BackendClient,
    kluster_id: &str,
    to_agent_id: Option<&str>,
    channel: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value> {
    use serde_json::json;
    let payload = json!({
        "to_agent_id": to_agent_id,
        "channel": channel,
        "body_json": body.to_string(),
    });
    client
        .post(&format!("/klusters/{kluster_id}/messages"), &payload)
        .await
}

/// Poll for new messages directed at this agent (in a kluster).
pub async fn poll_messages(
    client: &BackendClient,
    kluster_id: &str,
    since_id: Option<i64>,
) -> Result<Vec<serde_json::Value>> {
    let path = match since_id {
        Some(id) => format!("/klusters/{kluster_id}/messages?since_id={id}"),
        None => format!("/klusters/{kluster_id}/messages"),
    };
    client.get(&path).await
}
