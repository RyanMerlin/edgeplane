use edgeplaned_core::client::BackendClient;
use anyhow::Result;

/// Send a message scoped to a mission.
pub async fn send_mission_message(
    client: &BackendClient,
    mission_id: &str,
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
        .post(&format!("/missions/{mission_id}/messages"), &payload)
        .await
}

/// Poll for new messages directed at this agent (in a mission).
pub async fn poll_messages(
    client: &BackendClient,
    mission_id: &str,
    since_id: Option<i64>,
) -> Result<Vec<serde_json::Value>> {
    let path = match since_id {
        Some(id) => format!("/missions/{mission_id}/messages?since_id={id}"),
        None => format!("/missions/{mission_id}/messages"),
    };
    client.get(&path).await
}
