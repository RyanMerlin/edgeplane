use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver, Sender},
};

static JOB_COUNTER: AtomicU64 = AtomicU64::new(1);
pub type JobId = u64;

pub fn next_job_id() -> JobId {
    JOB_COUNTER.fetch_add(1, Ordering::Relaxed)
}

// ─── requests ────────────────────────────────────────────────────────────────

pub enum WorkRequest {
    /// Fetch the list of missions from the backend.
    ListMissions { job_id: JobId },
    /// Fetch klusters for a mission.
    ListKlusters { mission_id: String, job_id: JobId },
    /// Fetch tasks for a kluster using the canonical authenticated path.
    ListTasks { mission_id: String, kluster_id: String, job_id: JobId },
    /// Health-ping the backend; used for the status bar.
    Ping { job_id: JobId },
    /// Subscribe to the agent-feed SSE endpoint. The spawned thread streams
    /// events until the result channel closes or the connection drops.
    SubscribeFeed { base_url: String, token: Option<String> },
    /// List subfolder names at an Infisical path.
    LoadSecretFolders {
        job_id: JobId,
        project_id: String,
        environment: String,
        path: String,
        cfg: mc_mesh_secrets::InfisicalConfig,
    },
    /// List secret names (not values) at an Infisical path.
    LoadSecretNames {
        job_id: JobId,
        project_id: String,
        environment: String,
        path: String,
        cfg: mc_mesh_secrets::InfisicalConfig,
    },
    /// Fetch pending approvals for a mission (or all if mission_id is None).
    FetchApprovals { job_id: JobId, mission_id: Option<String> },
    /// Respond to a pending approval with "approve" or "reject".
    RespondApproval { job_id: JobId, approval_id: String, decision: String, note: Option<String> },
    /// Fetch the list of agents from the backend.
    ListAgents { job_id: JobId },
}

// ─── results ─────────────────────────────────────────────────────────────────

pub enum WorkResult {
    MissionsListed {
        job_id: JobId,
        missions: Vec<super::data::MissionSummary>,
        error: Option<String>,
    },
    KlustersListed {
        job_id: JobId,
        mission_id: String,
        klusters: Vec<super::data::KlusterSummary>,
        error: Option<String>,
    },
    TasksListed {
        job_id: JobId,
        kluster_id: String,
        tasks: Vec<super::data::TaskSummary>,
        error: Option<String>,
    },
    Pinged {
        job_id: JobId,
        ok: bool,
        latency_ms: u64,
        server_version: Option<String>,
    },
    /// An individual SSE event from the agent-feed stream.
    FeedEvent(super::screens::agent_feed::FeedEvent),
    /// The feed SSE connection is established (or re-established).
    FeedConnected,
    /// The feed SSE connection was lost; the caller should re-subscribe.
    FeedDisconnected { error: Option<String> },
    /// Subfolder names returned for a path.
    SecretFoldersLoaded { job_id: JobId, folders: Vec<String>, error: Option<String> },
    /// Secret names returned for a path.
    SecretNamesLoaded { job_id: JobId, names: Vec<String>, error: Option<String> },
    /// Pending approvals fetched.
    ApprovalsListed {
        job_id: JobId,
        approvals: Vec<super::data::ApprovalSummary>,
        error: Option<String>,
    },
    /// Approval respond call completed.
    ApprovalResponded { job_id: JobId, approval_id: String, ok: bool, error: Option<String> },
    /// Agents listed from the backend.
    AgentsListed { job_id: JobId, agents: Vec<super::data::AgentSummary>, error: Option<String> },
}

// ─── pool ────────────────────────────────────────────────────────────────────

pub struct WorkPool {
    result_tx: Sender<WorkResult>,
    pub result_rx: Receiver<WorkResult>,
}

impl WorkPool {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { result_tx: tx, result_rx: rx }
    }

    /// Dispatch a work request onto a background std::thread.
    ///
    /// The thread calls back into the tokio runtime via `Handle::current().block_on()`
    /// so async data fetches work without running inside the draw loop.
    pub fn dispatch(&self, client: std::sync::Arc<dyn super::data::DataClient>, req: WorkRequest) {
        let tx = self.result_tx.clone();
        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            match req {
                WorkRequest::SubscribeFeed { base_url, token } => {
                    handle.block_on(stream_feed(base_url, token, tx));
                }
                WorkRequest::Ping { job_id } => {
                    let start = std::time::Instant::now();
                    let result = handle.block_on(client.ping());
                    let latency_ms = start.elapsed().as_millis() as u64;
                    let (ok, server_version) = match result {
                        Ok(ver) => (true, ver),
                        Err(_) => (false, None),
                    };
                    let _ = tx.send(WorkResult::Pinged {
                        job_id,
                        ok,
                        latency_ms,
                        server_version,
                    });
                }
                WorkRequest::ListMissions { job_id } => {
                    match handle.block_on(client.list_missions()) {
                        Ok(missions) => {
                            let _ = tx.send(WorkResult::MissionsListed { job_id, missions, error: None });
                        }
                        Err(e) => {
                            let _ = tx.send(WorkResult::MissionsListed {
                                job_id,
                                missions: vec![],
                                error: Some(e.to_string()),
                            });
                        }
                    }
                }
                WorkRequest::ListKlusters { mission_id, job_id } => {
                    match handle.block_on(client.list_klusters(&mission_id)) {
                        Ok(klusters) => {
                            let _ = tx.send(WorkResult::KlustersListed { job_id, mission_id, klusters, error: None });
                        }
                        Err(e) => {
                            let _ = tx.send(WorkResult::KlustersListed {
                                job_id, mission_id, klusters: vec![], error: Some(e.to_string()),
                            });
                        }
                    }
                }
                WorkRequest::ListTasks { mission_id, kluster_id, job_id } => {
                    match handle.block_on(client.list_tasks(&mission_id, &kluster_id)) {
                        Ok(tasks) => {
                            let _ = tx.send(WorkResult::TasksListed { job_id, kluster_id, tasks, error: None });
                        }
                        Err(e) => {
                            let _ = tx.send(WorkResult::TasksListed {
                                job_id, kluster_id, tasks: vec![], error: Some(e.to_string()),
                            });
                        }
                    }
                }
                WorkRequest::LoadSecretFolders { job_id, project_id, environment, path, cfg } => {
                    // Infisical API requires no trailing slash except for root "/"
                    let api_path = if path == "/" { path.clone() } else { path.trim_end_matches('/').to_string() };
                    let infisical = mc_mesh_secrets::InfisicalClient::new(&cfg);
                    match infisical {
                        Err(e) => {
                            let _ = tx.send(WorkResult::SecretFoldersLoaded {
                                job_id, folders: vec![], error: Some(e.to_string()),
                            });
                        }
                        Ok(c) => {
                            match handle.block_on(c.list_folders(&project_id, &environment, &api_path)) {
                                Ok(folders) => {
                                    let _ = tx.send(WorkResult::SecretFoldersLoaded { job_id, folders, error: None });
                                }
                                Err(e) => {
                                    let _ = tx.send(WorkResult::SecretFoldersLoaded {
                                        job_id, folders: vec![], error: Some(e.to_string()),
                                    });
                                }
                            }
                        }
                    }
                }
                WorkRequest::LoadSecretNames { job_id, project_id, environment, path, cfg } => {
                    // Infisical API requires no trailing slash except for root "/"
                    let api_path = if path == "/" { path.clone() } else { path.trim_end_matches('/').to_string() };
                    let infisical = mc_mesh_secrets::InfisicalClient::new(&cfg);
                    match infisical {
                        Err(e) => {
                            let _ = tx.send(WorkResult::SecretNamesLoaded {
                                job_id, names: vec![], error: Some(e.to_string()),
                            });
                        }
                        Ok(c) => {
                            match handle.block_on(c.list_secrets(&project_id, &environment, &api_path)) {
                                Ok(names) => {
                                    let _ = tx.send(WorkResult::SecretNamesLoaded { job_id, names, error: None });
                                }
                                Err(e) => {
                                    let _ = tx.send(WorkResult::SecretNamesLoaded {
                                        job_id, names: vec![], error: Some(e.to_string()),
                                    });
                                }
                            }
                        }
                    }
                }
                WorkRequest::FetchApprovals { job_id, mission_id } => {
                    match handle.block_on(client.list_approvals(mission_id.as_deref())) {
                        Ok(approvals) => {
                            let _ = tx.send(WorkResult::ApprovalsListed { job_id, approvals, error: None });
                        }
                        Err(e) => {
                            let _ = tx.send(WorkResult::ApprovalsListed {
                                job_id, approvals: vec![], error: Some(e.to_string()),
                            });
                        }
                    }
                }
                WorkRequest::RespondApproval { job_id, approval_id, decision, note } => {
                    let res = handle.block_on(
                        client.respond_approval(&approval_id, &decision, note.as_deref())
                    );
                    let (ok, error) = match res {
                        Ok(()) => (true, None),
                        Err(e) => (false, Some(e.to_string())),
                    };
                    let _ = tx.send(WorkResult::ApprovalResponded { job_id, approval_id, ok, error });
                }
                WorkRequest::ListAgents { job_id } => {
                    match handle.block_on(client.list_agents()) {
                        Ok(agents) => { let _ = tx.send(WorkResult::AgentsListed { job_id, agents, error: None }); }
                        Err(e) => { let _ = tx.send(WorkResult::AgentsListed { job_id, agents: vec![], error: Some(e.to_string()) }); }
                    }
                }
            }
        });
    }
}

impl Default for WorkPool {
    fn default() -> Self { Self::new() }
}

/// Connect to the backend's agent-feed SSE endpoint and stream events until the
/// channel closes or the connection drops.  Sends WorkResult::FeedConnected on
/// first successful connect, then one WorkResult::FeedEvent per parsed event,
/// then WorkResult::FeedDisconnected on disconnect.
async fn stream_feed(base_url: String, token: Option<String>, tx: std::sync::mpsc::Sender<WorkResult>) {
    use futures_util::StreamExt;

    let url = format!("{}/sse", base_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(WorkResult::FeedDisconnected { error: Some(e.to_string()) });
            return;
        }
    };

    let mut req = client.get(&url).header("Accept", "text/event-stream");
    if let Some(tok) = &token {
        req = req.bearer_auth(tok);
    }

    let resp = match req.send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            let _ = tx.send(WorkResult::FeedDisconnected {
                error: Some(format!("backend returned {}", r.status())),
            });
            return;
        }
        Err(e) => {
            let _ = tx.send(WorkResult::FeedDisconnected { error: Some(e.to_string()) });
            return;
        }
    };

    let _ = tx.send(WorkResult::FeedConnected);

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut current_event_type = String::from("message");
    let mut current_data = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(WorkResult::FeedDisconnected { error: Some(e.to_string()) });
                return;
            }
        };
        buf.push_str(&String::from_utf8_lossy(&chunk));

        // SSE: events are newline-delimited
        while let Some(newline_pos) = buf.find('\n') {
            let line = buf[..newline_pos].trim_end_matches('\r').to_string();
            buf.drain(..newline_pos + 1);

            if line.is_empty() {
                // Empty line = dispatch event
                if !current_data.is_empty() {
                    let ts = chrono::Utc::now().format("%H:%M:%S%.3f").to_string();
                    let (agent_id, mission_id, evdata) =
                        parse_feed_data(&current_data);
                    let ev = super::screens::agent_feed::FeedEvent {
                        ts,
                        agent_id,
                        mission_id,
                        event_type: current_event_type.clone(),
                        data: evdata,
                    };
                    if tx.send(WorkResult::FeedEvent(ev)).is_err() {
                        return; // channel closed, app is gone
                    }
                }
                current_data.clear();
                current_event_type = "message".to_string();
            } else if let Some(data) = line.strip_prefix("data: ") {
                current_data.push_str(data);
            } else if let Some(etype) = line.strip_prefix("event: ") {
                current_event_type = etype.to_string();
            }
            // ignore `id:` and `retry:` lines
        }
    }

    let _ = tx.send(WorkResult::FeedDisconnected { error: None });
}

/// Try to parse agent_id / mission_id from the SSE data payload (expected to be JSON).
fn parse_feed_data(data: &str) -> (Option<String>, Option<String>, String) {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
        let agent_id = v.get("agent_id").and_then(|x| x.as_str()).map(str::to_string)
            .or_else(|| v.get("agent").and_then(|x| x.as_str()).map(str::to_string));
        let mission_id = v.get("mission_id").and_then(|x| x.as_str()).map(str::to_string);
        let summary = v.get("message").or_else(|| v.get("data")).or_else(|| v.get("summary"))
            .and_then(|x| x.as_str()).unwrap_or(data).to_string();
        (agent_id, mission_id, summary)
    } else {
        (None, None, data.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::data::{DataClient, FixtureDataClient, MissionSummary};
    use std::sync::Arc;

    #[tokio::test]
    async fn pool_delivers_ping_result() {
        let pool = WorkPool::new();
        let client: Arc<dyn DataClient> = Arc::new(FixtureDataClient::default());
        let job_id = next_job_id();
        pool.dispatch(client, WorkRequest::Ping { job_id });
        let result = pool.result_rx.recv_timeout(std::time::Duration::from_secs(5));
        assert!(result.is_ok(), "no result arrived");
        match result.unwrap() {
            WorkResult::Pinged { job_id: jid, ok, .. } => {
                assert_eq!(jid, job_id);
                assert!(ok);
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[tokio::test]
    async fn pool_delivers_missions_result() {
        let pool = WorkPool::new();
        let client: Arc<dyn DataClient> = Arc::new(FixtureDataClient {
            missions: vec![MissionSummary {
                id: "m1".into(),
                name: "Test Mission".into(),
                status: "active".into(),
            }],
        });
        let job_id = next_job_id();
        pool.dispatch(client, WorkRequest::ListMissions { job_id });
        let result = pool.result_rx.recv_timeout(std::time::Duration::from_secs(5));
        assert!(result.is_ok());
        match result.unwrap() {
            WorkResult::MissionsListed { missions, error, .. } => {
                assert!(error.is_none());
                assert_eq!(missions.len(), 1);
                assert_eq!(missions[0].name, "Test Mission");
            }
            _ => panic!("unexpected variant"),
        }
    }
}
