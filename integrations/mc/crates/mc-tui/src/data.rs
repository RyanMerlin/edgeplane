use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize};

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
}

// ─── remote client (wraps reqwest, talks to mc-server / backend) ──────────────

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
        if !status.is_success() {
            anyhow::bail!("backend returned {status} for /agents");
        }
        Ok(resp.json::<Vec<AgentSummary>>().await?)
    }
}
