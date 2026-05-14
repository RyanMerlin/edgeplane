use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

/// Render an ISO-8601 timestamp as a coarse "X ago" string. Returns the input
/// string unchanged if it can't be parsed, so callers don't need to fall back.
pub fn humanize_since(iso: &str) -> String {
    let parsed = DateTime::parse_from_rfc3339(iso)
        .map(|d| d.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S%.f")
                .map(|n| DateTime::<Utc>::from_naive_utc_and_offset(n, Utc))
        })
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%d %H:%M:%S%.f")
                .map(|n| DateTime::<Utc>::from_naive_utc_and_offset(n, Utc))
        });
    let Ok(t) = parsed else { return iso.to_string() };
    let secs = (Utc::now() - t).num_seconds();
    if secs < 0 {
        // Clock skew or future timestamp — display raw.
        return iso.to_string();
    }
    if secs < 5            { return "just now".to_string(); }
    if secs < 60           { return format!("{}s ago", secs); }
    let mins = secs / 60;
    if mins < 60           { return format!("{}m ago", mins); }
    let hours = mins / 60;
    if hours < 24          { return format!("{}h ago", hours); }
    let days = hours / 24;
    if days < 30           { return format!("{}d ago", days); }
    let months = days / 30;
    if months < 12         { return format!("{}mo ago", months); }
    format!("{}y ago", months / 12)
}

// ─── domain types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionSummary {
    pub id: String,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KlusterSummary {
    pub id: String,
    #[serde(default)]
    pub mission_id: Option<String>,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummary {
    pub id: i32,
    pub public_id: String,
    pub kluster_id: String,
    pub title: String,
    pub status: String,
    pub owner: String,
    #[serde(default)]
    pub description: String,
}

// ─── approvals ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalSummary {
    pub id: i64,
    #[serde(default)]
    pub mission_id: Option<String>,
    pub action: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub requested_by: Option<String>,
    pub status: String,
    #[serde(default)]
    pub request_context: Option<serde_json::Value>,
}

// ─── agent summary ───────────────────────────────────────────────────────────

fn id_to_string<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    use serde::de::Error;
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        other => Err(D::Error::custom(format!("expected string or number for id, got {other}"))),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSummary {
    #[serde(deserialize_with = "id_to_string")]
    pub id: String,
    /// Stable wire identifier — `{name}-{8 hex}`. Preferred over `id` for
    /// any caller-facing surface; falls back to `id` when the server has
    /// not yet populated it. Introduced by the agent-public-id migration
    /// (`docs/plans/2026-05-11-agent-public-id-mc-mesh-fix.md`).
    #[serde(default)]
    pub public_id: Option<String>,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub capabilities: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub mission_id: Option<String>,
    #[serde(default)]
    pub mission_name: Option<String>,
    #[serde(default)]
    pub current_task_title: Option<String>,
    #[serde(default)]
    pub last_seen: Option<String>,
    /// Raw metadata JSON string from the server; derived fields are unpacked from here.
    #[serde(default)]
    pub metadata: Option<String>,
}

impl AgentSummary {
    /// Unpack the `metadata` JSON string into derived fields the server doesn't expose top-level.
    pub fn resolve_metadata(&mut self) {
        let raw = match &self.metadata {
            Some(s) if !s.is_empty() && s != "{}" => s.clone(),
            _ => return,
        };
        let Ok(meta) = serde_json::from_str::<serde_json::Value>(&raw) else { return };
        if self.runtime.is_none() { self.runtime = meta["runtime"].as_str().map(String::from); }
        if self.node_id.is_none() { self.node_id = meta["node_id"].as_str().map(String::from); }
        if self.mission_name.is_none() { self.mission_name = meta["mission_name"].as_str().map(String::from); }
        if self.current_task_title.is_none() { self.current_task_title = meta["current_task"].as_str().map(String::from); }
        if self.last_seen.is_none() { self.last_seen = meta["last_seen"].as_str().map(String::from); }
    }
}

// ─── trait ───────────────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait DataClient: Send + Sync {
    async fn ping(&self) -> Result<Option<String>>;
    async fn list_missions(&self) -> Result<Vec<MissionSummary>>;
    async fn list_klusters(&self, mission_id: &str) -> Result<Vec<KlusterSummary>>;
    async fn list_tasks(&self, mission_id: &str, kluster_id: &str) -> Result<Vec<TaskSummary>>;
    async fn list_approvals(&self, mission_id: Option<&str>) -> Result<Vec<ApprovalSummary>>;
    async fn respond_approval(&self, approval_id: &str, decision: &str, note: Option<&str>) -> Result<()>;
    async fn list_agents(&self) -> Result<Vec<AgentSummary>>;
    async fn delete_agent(&self, agent_id: &str) -> Result<()>;
    async fn restart_agent(&self, agent_id: &str) -> Result<()>;
    async fn clear_agent_context(&self, agent_id: &str) -> Result<()>;
    /// Resolve the current token to a subject via /auth/whoami. Returns None on error.
    async fn whoami(&self) -> Result<Option<String>>;
}

// ─── fixture client (test / offline use) ─────────────────────────────────────

#[derive(Default)]
pub struct FixtureDataClient {
    pub missions: Vec<MissionSummary>,
}

#[async_trait::async_trait]
impl DataClient for FixtureDataClient {
    async fn ping(&self) -> Result<Option<String>> { Ok(None) }

    async fn list_missions(&self) -> Result<Vec<MissionSummary>> {
        Ok(self.missions.clone())
    }

    async fn list_klusters(&self, _mission_id: &str) -> Result<Vec<KlusterSummary>> {
        Ok(vec![])
    }

    async fn list_tasks(&self, _mission_id: &str, _kluster_id: &str) -> Result<Vec<TaskSummary>> {
        Ok(vec![])
    }

    async fn list_approvals(&self, _mission_id: Option<&str>) -> Result<Vec<ApprovalSummary>> {
        Ok(vec![])
    }

    async fn respond_approval(&self, _approval_id: &str, _decision: &str, _note: Option<&str>) -> Result<()> {
        Ok(())
    }

    async fn list_agents(&self) -> Result<Vec<AgentSummary>> {
        Ok(vec![])
    }

    async fn delete_agent(&self, _agent_id: &str) -> Result<()> {
        Ok(())
    }

    async fn restart_agent(&self, _agent_id: &str) -> Result<()> {
        Ok(())
    }

    async fn clear_agent_context(&self, _agent_id: &str) -> Result<()> {
        Ok(())
    }

    async fn whoami(&self) -> Result<Option<String>> {
        Ok(None)
    }
}

// ─── remote client (wraps reqwest, talks to mc-controlplane) ──────────────────

/// Sentinel prefix used in error messages when the controlplane returns 401.
/// `tick()` matches on this to switch the app into the SessionExpired state
/// without each callsite having to plumb a typed error all the way up.
pub const AUTH_ERROR_PREFIX: &str = "unauthorized";

/// True iff the error string was produced by a 401 response. Cheap and
/// allocation-free; safe to call on every poll.
pub fn is_auth_error(msg: &str) -> bool {
    msg.starts_with(AUTH_ERROR_PREFIX)
}

pub struct RemoteDataClient {
    pub base_url: String,
    pub token: Option<String>,
    client: reqwest::Client,
}

impl RemoteDataClient {
    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        Ok(Self { base_url: base_url.into(), token, client })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let mut req = self.client.get(self.url(path));
        if let Some(tok) = &self.token {
            req = req.bearer_auth(tok);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            anyhow::bail!("{AUTH_ERROR_PREFIX}: session missing or expired ({path})");
        }
        if !status.is_success() {
            anyhow::bail!("backend returned {status} for {path}");
        }
        Ok(resp.json::<T>().await?)
    }
}

#[async_trait::async_trait]
impl DataClient for RemoteDataClient {
    async fn ping(&self) -> Result<Option<String>> {
        let v = self.get::<serde_json::Value>("/health").await?;
        let ver = v.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
        Ok(ver)
    }

    async fn list_missions(&self) -> Result<Vec<MissionSummary>> {
        self.get("/missions").await
    }

    async fn list_klusters(&self, mission_id: &str) -> Result<Vec<KlusterSummary>> {
        self.get(&format!("/missions/{mission_id}/k")).await
    }

    // Uses the canonical auth-required path rather than the /klusters/:id/t shortcut.
    async fn list_tasks(&self, mission_id: &str, kluster_id: &str) -> Result<Vec<TaskSummary>> {
        self.get(&format!("/missions/{mission_id}/k/{kluster_id}/t")).await
    }

    async fn list_approvals(&self, mission_id: Option<&str>) -> Result<Vec<ApprovalSummary>> {
        let path = if let Some(mid) = mission_id {
            format!("/approvals?mission_id={mid}&status=pending")
        } else {
            "/approvals?status=pending".to_string()
        };
        self.get(&path).await
    }

    async fn respond_approval(&self, approval_id: &str, decision: &str, note: Option<&str>) -> Result<()> {
        let mut req = self.client.post(self.url(&format!("/approvals/{approval_id}/respond")));
        if let Some(tok) = &self.token {
            req = req.bearer_auth(tok);
        }
        let body = serde_json::json!({"decision": decision, "note": note.unwrap_or("")});
        let resp = req.json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("respond_approval returned {status}: {text}");
        }
        Ok(())
    }

    async fn list_agents(&self) -> Result<Vec<AgentSummary>> {
        let mut req = self.client.get(self.url("/agents"));
        if let Some(tok) = &self.token {
            req = req.bearer_auth(tok);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(vec![]);
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            anyhow::bail!("{AUTH_ERROR_PREFIX}: session missing or expired (/agents)");
        }
        if !status.is_success() {
            anyhow::bail!("backend returned {status} for /agents");
        }
        Ok(resp.json::<Vec<AgentSummary>>().await?)
    }

    async fn delete_agent(&self, agent_id: &str) -> Result<()> {
        let mut req = self.client.delete(self.url(&format!("/agents/{agent_id}")));
        if let Some(tok) = &self.token {
            req = req.bearer_auth(tok);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("delete_agent returned {status}: {text}");
        }
        Ok(())
    }

    async fn restart_agent(&self, agent_id: &str) -> Result<()> {
        let mut req = self.client.post(self.url(&format!("/agents/{agent_id}/restart")));
        if let Some(tok) = &self.token {
            req = req.bearer_auth(tok);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("restart_agent returned {status}: {text}");
        }
        Ok(())
    }

    async fn clear_agent_context(&self, agent_id: &str) -> Result<()> {
        let mut req = self.client.post(self.url(&format!("/agents/{agent_id}/clear-context")));
        if let Some(tok) = &self.token {
            req = req.bearer_auth(tok);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("clear_agent_context returned {status}: {text}");
        }
        Ok(())
    }

    async fn whoami(&self) -> Result<Option<String>> {
        let v = self.get::<serde_json::Value>("/auth/whoami").await?;
        Ok(v.get("subject").and_then(|s| s.as_str()).map(String::from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn iso(offset: Duration) -> String {
        (Utc::now() + offset).to_rfc3339()
    }

    #[test]
    fn humanize_recent_seconds() {
        assert_eq!(humanize_since(&iso(-Duration::seconds(2))), "just now");
        assert_eq!(humanize_since(&iso(-Duration::seconds(30))), "30s ago");
    }

    #[test]
    fn humanize_minutes_hours_days() {
        assert_eq!(humanize_since(&iso(-Duration::minutes(5))), "5m ago");
        assert_eq!(humanize_since(&iso(-Duration::hours(3))), "3h ago");
        assert_eq!(humanize_since(&iso(-Duration::days(2))), "2d ago");
    }

    #[test]
    fn humanize_unparseable_returns_input() {
        assert_eq!(humanize_since("not a date"), "not a date");
    }

    #[test]
    fn humanize_naive_postgres_timestamp() {
        // Postgres often serializes timestamp without TZ as "2024-01-02T03:04:05.123"
        // — we treat those as UTC and still produce a relative string.
        let now = Utc::now() - Duration::minutes(2);
        let s = now.format("%Y-%m-%dT%H:%M:%S%.3f").to_string();
        let out = humanize_since(&s);
        assert!(out.ends_with("ago") || out == "just now", "got {out}");
    }
}
