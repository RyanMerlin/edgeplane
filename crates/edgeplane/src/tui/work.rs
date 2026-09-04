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
    /// Fetch the list of domains from the backend.
    ListDomains { job_id: JobId },
    /// Fetch missions for a domain.
    ListMissions { domain_id: String, job_id: JobId },
    /// Fetch tasks for a mission using the canonical authenticated path.
    ListTasks {
        domain_id: String,
        mission_id: String,
        job_id: JobId,
    },
    /// Health-ping the backend; used for the status bar.
    Ping { job_id: JobId },
    /// Subscribe to the agent-feed SSE endpoint. The spawned thread streams
    /// events until the result channel closes or the connection drops.
    SubscribeFeed {
        base_url: String,
        token: Option<String>,
    },
    /// List subfolder names at an Infisical path.
    LoadSecretFolders {
        job_id: JobId,
        project_id: String,
        environment: String,
        path: String,
        cfg: edgeplaned_secrets::InfisicalConfig,
    },
    /// List secret names (not values) at an Infisical path.
    LoadSecretNames {
        job_id: JobId,
        project_id: String,
        environment: String,
        path: String,
        cfg: edgeplaned_secrets::InfisicalConfig,
    },
    /// Fetch the list of agents from the backend.
    ListAgents { job_id: JobId },
    /// Delete an agent by id.
    DeleteAgent { job_id: JobId, agent_id: String },
    /// Restart an agent — controlplane ends sessions and signals; the runtime acts.
    RestartAgent { job_id: JobId, agent_id: String },
    /// Clear an agent's context — controlplane stamps metadata; the runtime acts.
    ClearAgentContext { job_id: JobId, agent_id: String },
    /// Run the full OIDC browser-based login flow in-TUI.
    OidcFlow {
        job_id: JobId,
        base_url: String,
        ttl_hours: u64,
    },
    /// Ping a URL to test connectivity (unauthenticated GET {url}/health).
    PingUrl { job_id: JobId, url: String },
    /// Resolve the current token to a subject via /auth/whoami.
    Whoami { job_id: JobId },
}

// ─── results ─────────────────────────────────────────────────────────────────

pub enum WorkResult {
    DomainsListed {
        job_id: JobId,
        domains: Vec<super::data::DomainSummary>,
        error: Option<String>,
    },
    MissionsListed {
        job_id: JobId,
        domain_id: String,
        missions: Vec<super::data::MissionSummary>,
        error: Option<String>,
    },
    TasksListed {
        job_id: JobId,
        mission_id: String,
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
    SecretFoldersLoaded {
        job_id: JobId,
        folders: Vec<String>,
        error: Option<String>,
    },
    /// Secret names returned for a path.
    SecretNamesLoaded {
        job_id: JobId,
        names: Vec<String>,
        error: Option<String>,
    },
    /// Agents listed from the backend.
    AgentsListed {
        job_id: JobId,
        agents: Vec<super::data::AgentSummary>,
        error: Option<String>,
    },
    /// Agent delete completed.
    AgentDeleted {
        job_id: JobId,
        agent_id: String,
        ok: bool,
        error: Option<String>,
    },
    /// Agent op (restart / clear-context) completed.
    AgentOpCompleted {
        job_id: JobId,
        agent_id: String,
        op: &'static str,
        ok: bool,
        error: Option<String>,
    },
    /// An event from the in-TUI OIDC login flow.
    OidcFlow(OidcFlowEvent),
    /// Result of a PingUrl work item.
    UrlTested {
        job_id: JobId,
        url: String,
        ok: bool,
        latency_ms: u64,
        version: Option<String>,
        error: Option<String>,
    },
    /// Subject resolved from /auth/whoami for a token-authenticated session.
    WhoamiComplete {
        job_id: JobId,
        subject: Option<String>,
    },
}

/// Events emitted during an in-TUI OIDC browser login flow.
pub enum OidcFlowEvent {
    /// Server accepted initiation; browser URL is ready to display / open.
    Initiated {
        job_id: JobId,
        authorize_url: String,
    },
    /// Browser flow completed and session token saved.
    Complete {
        job_id: JobId,
        token: String,
        subject: String,
        expires_at: String,
        email: Option<String>,
    },
    /// Polling for browser completion timed out (60 s).
    TimedOut { job_id: JobId },
    /// Any unrecoverable error during the flow.
    Failed { job_id: JobId, error: String },
}

// ─── pool ────────────────────────────────────────────────────────────────────

pub struct WorkPool {
    result_tx: Sender<WorkResult>,
    pub result_rx: Receiver<WorkResult>,
}

impl WorkPool {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            result_tx: tx,
            result_rx: rx,
        }
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
                WorkRequest::ListDomains { job_id } => {
                    match handle.block_on(client.list_domains()) {
                        Ok(domains) => {
                            let _ = tx.send(WorkResult::DomainsListed {
                                job_id,
                                domains,
                                error: None,
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(WorkResult::DomainsListed {
                                job_id,
                                domains: vec![],
                                error: Some(e.to_string()),
                            });
                        }
                    }
                }
                WorkRequest::ListMissions { domain_id, job_id } => {
                    match handle.block_on(client.list_missions(&domain_id)) {
                        Ok(missions) => {
                            let _ = tx.send(WorkResult::MissionsListed {
                                job_id,
                                domain_id,
                                missions,
                                error: None,
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(WorkResult::MissionsListed {
                                job_id,
                                domain_id,
                                missions: vec![],
                                error: Some(e.to_string()),
                            });
                        }
                    }
                }
                WorkRequest::ListTasks {
                    domain_id,
                    mission_id,
                    job_id,
                } => match handle.block_on(client.list_tasks(&domain_id, &mission_id)) {
                    Ok(tasks) => {
                        let _ = tx.send(WorkResult::TasksListed {
                            job_id,
                            mission_id,
                            tasks,
                            error: None,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(WorkResult::TasksListed {
                            job_id,
                            mission_id,
                            tasks: vec![],
                            error: Some(e.to_string()),
                        });
                    }
                },
                WorkRequest::LoadSecretFolders {
                    job_id,
                    project_id,
                    environment,
                    path,
                    cfg,
                } => {
                    // Infisical API requires no trailing slash except for root "/"
                    let api_path = if path == "/" {
                        path.clone()
                    } else {
                        path.trim_end_matches('/').to_string()
                    };
                    let infisical = edgeplaned_secrets::InfisicalClient::new(&cfg);
                    match infisical {
                        Err(e) => {
                            let _ = tx.send(WorkResult::SecretFoldersLoaded {
                                job_id,
                                folders: vec![],
                                error: Some(e.to_string()),
                            });
                        }
                        Ok(c) => {
                            match handle.block_on(c.list_folders(
                                &project_id,
                                &environment,
                                &api_path,
                            )) {
                                Ok(folders) => {
                                    let _ = tx.send(WorkResult::SecretFoldersLoaded {
                                        job_id,
                                        folders,
                                        error: None,
                                    });
                                }
                                Err(e) => {
                                    let _ = tx.send(WorkResult::SecretFoldersLoaded {
                                        job_id,
                                        folders: vec![],
                                        error: Some(e.to_string()),
                                    });
                                }
                            }
                        }
                    }
                }
                WorkRequest::LoadSecretNames {
                    job_id,
                    project_id,
                    environment,
                    path,
                    cfg,
                } => {
                    // Infisical API requires no trailing slash except for root "/"
                    let api_path = if path == "/" {
                        path.clone()
                    } else {
                        path.trim_end_matches('/').to_string()
                    };
                    let infisical = edgeplaned_secrets::InfisicalClient::new(&cfg);
                    match infisical {
                        Err(e) => {
                            let _ = tx.send(WorkResult::SecretNamesLoaded {
                                job_id,
                                names: vec![],
                                error: Some(e.to_string()),
                            });
                        }
                        Ok(c) => {
                            match handle.block_on(c.list_secrets(
                                &project_id,
                                &environment,
                                &api_path,
                            )) {
                                Ok(names) => {
                                    let _ = tx.send(WorkResult::SecretNamesLoaded {
                                        job_id,
                                        names,
                                        error: None,
                                    });
                                }
                                Err(e) => {
                                    let _ = tx.send(WorkResult::SecretNamesLoaded {
                                        job_id,
                                        names: vec![],
                                        error: Some(e.to_string()),
                                    });
                                }
                            }
                        }
                    }
                }
                WorkRequest::ListAgents { job_id } => match handle.block_on(client.list_agents()) {
                    Ok(agents) => {
                        let _ = tx.send(WorkResult::AgentsListed {
                            job_id,
                            agents,
                            error: None,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(WorkResult::AgentsListed {
                            job_id,
                            agents: vec![],
                            error: Some(e.to_string()),
                        });
                    }
                },
                WorkRequest::DeleteAgent { job_id, agent_id } => {
                    let res = handle.block_on(client.delete_agent(&agent_id));
                    let (ok, error) = match res {
                        Ok(()) => (true, None),
                        Err(e) => (false, Some(e.to_string())),
                    };
                    let _ = tx.send(WorkResult::AgentDeleted {
                        job_id,
                        agent_id,
                        ok,
                        error,
                    });
                }
                WorkRequest::RestartAgent { job_id, agent_id } => {
                    let res = handle.block_on(client.restart_agent(&agent_id));
                    let (ok, error) = match res {
                        Ok(()) => (true, None),
                        Err(e) => (false, Some(e.to_string())),
                    };
                    let _ = tx.send(WorkResult::AgentOpCompleted {
                        job_id,
                        agent_id,
                        op: "restart",
                        ok,
                        error,
                    });
                }
                WorkRequest::ClearAgentContext { job_id, agent_id } => {
                    let res = handle.block_on(client.clear_agent_context(&agent_id));
                    let (ok, error) = match res {
                        Ok(()) => (true, None),
                        Err(e) => (false, Some(e.to_string())),
                    };
                    let _ = tx.send(WorkResult::AgentOpCompleted {
                        job_id,
                        agent_id,
                        op: "clear-context",
                        ok,
                        error,
                    });
                }
                WorkRequest::OidcFlow {
                    job_id,
                    base_url,
                    ttl_hours,
                } => {
                    handle.block_on(oidc_flow_worker(job_id, base_url, ttl_hours, tx));
                }
                WorkRequest::PingUrl { job_id, url } => {
                    let start = std::time::Instant::now();
                    let result = handle.block_on(async {
                        let ping_client = reqwest::Client::builder()
                            .timeout(std::time::Duration::from_secs(5))
                            .build()
                            .map_err(|e| e.to_string())?;
                        let health_url = format!("{}/health", url.trim_end_matches('/'));
                        let resp = ping_client
                            .get(&health_url)
                            .send()
                            .await
                            .map_err(|e| e.to_string())?;
                        let status = resp.status();
                        if status.is_success() {
                            let body: serde_json::Value = resp.json().await.unwrap_or_default();
                            let version = body["version"].as_str().map(str::to_string);
                            Ok(version)
                        } else {
                            Err(format!("server returned {status}"))
                        }
                    });
                    let latency_ms = start.elapsed().as_millis() as u64;
                    match result {
                        Ok(version) => {
                            let _ = tx.send(WorkResult::UrlTested {
                                job_id,
                                url,
                                ok: true,
                                latency_ms,
                                version,
                                error: None,
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(WorkResult::UrlTested {
                                job_id,
                                url,
                                ok: false,
                                latency_ms: 0,
                                version: None,
                                error: Some(e),
                            });
                        }
                    }
                }
                WorkRequest::Whoami { job_id } => {
                    let subject = handle.block_on(client.whoami()).ok().flatten();
                    let _ = tx.send(WorkResult::WhoamiComplete { job_id, subject });
                }
            }
        });
    }
}

impl Default for WorkPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Run the full OIDC browser-based login flow: initiate → open browser → poll
/// for completion → exchange grant for a session token → save session to disk.
/// Sends OidcFlowEvent variants back on `tx` as the flow progresses.
async fn oidc_flow_worker(
    job_id: JobId,
    base_url: String,
    ttl_hours: u64,
    tx: std::sync::mpsc::Sender<WorkResult>,
) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(WorkResult::OidcFlow(OidcFlowEvent::Failed {
                job_id,
                error: format!("could not build HTTP client: {e}"),
            }));
            return;
        }
    };

    let base = base_url.trim_end_matches('/').to_string();

    // Step 1: initiate
    let init_url = format!("{base}/auth/oidc/cli-initiate");
    let init_resp = match client.get(&init_url).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            let _ = tx.send(WorkResult::OidcFlow(OidcFlowEvent::Failed {
                job_id,
                error: format!("OIDC initiate returned {}", r.status()),
            }));
            return;
        }
        Err(e) => {
            let _ = tx.send(WorkResult::OidcFlow(OidcFlowEvent::Failed {
                job_id,
                error: format!("OIDC initiate request failed: {e}"),
            }));
            return;
        }
    };

    let init_json: serde_json::Value = match init_resp.json().await {
        Ok(v) => v,
        Err(e) => {
            let _ = tx.send(WorkResult::OidcFlow(OidcFlowEvent::Failed {
                job_id,
                error: format!("failed to parse initiate response: {e}"),
            }));
            return;
        }
    };

    let authorize_url = match init_json["authorize_url"].as_str() {
        Some(u) => u.to_string(),
        None => {
            let _ = tx.send(WorkResult::OidcFlow(OidcFlowEvent::Failed {
                job_id,
                error: "server returned no authorize_url".to_string(),
            }));
            return;
        }
    };
    let cli_nonce = match init_json["cli_nonce"].as_str() {
        Some(n) => n.to_string(),
        None => {
            let _ = tx.send(WorkResult::OidcFlow(OidcFlowEvent::Failed {
                job_id,
                error: "server returned no cli_nonce".to_string(),
            }));
            return;
        }
    };

    // Notify the TUI that we have a URL to display
    let _ = tx.send(WorkResult::OidcFlow(OidcFlowEvent::Initiated {
        job_id,
        authorize_url: authorize_url.clone(),
    }));

    // Best-effort browser open
    let _ = open::that(&authorize_url);

    // Step 2: poll for browser completion (60 s deadline, 2 s interval)
    let poll_url = format!("{base}/auth/oidc/cli-poll/{cli_nonce}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let grant_id = loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if std::time::Instant::now() >= deadline {
            let _ = tx.send(WorkResult::OidcFlow(OidcFlowEvent::TimedOut { job_id }));
            return;
        }
        match client.get(&poll_url).send().await {
            Ok(r) if r.status().is_success() => {
                if let Ok(v) = r.json::<serde_json::Value>().await
                    && v["status"].as_str() == Some("ready")
                    && let Some(gid) = v["grant_id"].as_str()
                {
                    break gid.to_string();
                }
            }
            _ => {} // transient error or still pending — keep polling
        }
    };

    // Step 3: exchange grant for session token
    let exchange_url = format!("{base}/auth/oidc/exchange");
    let ttl = ttl_hours.clamp(1, 8760);
    let exchange_resp = match client
        .post(&exchange_url)
        .json(&serde_json::json!({ "grant_id": grant_id, "ttl_hours": ttl }))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            let _ = tx.send(WorkResult::OidcFlow(OidcFlowEvent::Failed {
                job_id,
                error: format!("exchange returned {status}: {body}"),
            }));
            return;
        }
        Err(e) => {
            let _ = tx.send(WorkResult::OidcFlow(OidcFlowEvent::Failed {
                job_id,
                error: format!("exchange request failed: {e}"),
            }));
            return;
        }
    };

    let resp_json: serde_json::Value = match exchange_resp.json().await {
        Ok(v) => v,
        Err(e) => {
            let _ = tx.send(WorkResult::OidcFlow(OidcFlowEvent::Failed {
                job_id,
                error: format!("failed to parse exchange response: {e}"),
            }));
            return;
        }
    };

    let token = match resp_json["token"]
        .as_str()
        .or_else(|| resp_json["access_token"].as_str())
    {
        Some(t) => t.to_string(),
        None => {
            let _ = tx.send(WorkResult::OidcFlow(OidcFlowEvent::Failed {
                job_id,
                error: "exchange response missing token field".to_string(),
            }));
            return;
        }
    };
    let subject = resp_json["subject"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let email = resp_json["email"].as_str().map(|s| s.to_string());
    let expires_at = resp_json["expires_at"].as_str().unwrap_or("").to_string();
    let session_id = resp_json["session_id"].as_i64();

    let session = crate::auth::SavedSession {
        token: token.clone(),
        subject: subject.clone(),
        email: email.clone(),
        expires_at: expires_at.clone(),
        base_url: base,
        session_id,
    };
    if let Err(e) = crate::auth::save_session(&session) {
        let _ = tx.send(WorkResult::OidcFlow(OidcFlowEvent::Failed {
            job_id,
            error: format!("failed to save session: {e}"),
        }));
        return;
    }

    let _ = tx.send(WorkResult::OidcFlow(OidcFlowEvent::Complete {
        job_id,
        token,
        subject,
        expires_at,
        email,
    }));
}

/// Connect to the backend's agent-feed SSE endpoint and stream events until the
/// channel closes or the connection drops.  Sends WorkResult::FeedConnected on
/// first successful connect, then one WorkResult::FeedEvent per parsed event,
/// then WorkResult::FeedDisconnected on disconnect.
async fn stream_feed(
    base_url: String,
    token: Option<String>,
    tx: std::sync::mpsc::Sender<WorkResult>,
) {
    use futures_util::StreamExt;

    let url = format!("{}/sse", base_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(WorkResult::FeedDisconnected {
                error: Some(e.to_string()),
            });
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
            let _ = tx.send(WorkResult::FeedDisconnected {
                error: Some(e.to_string()),
            });
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
                let _ = tx.send(WorkResult::FeedDisconnected {
                    error: Some(e.to_string()),
                });
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
                    let (agent_id, domain_id, evdata) = parse_feed_data(&current_data);
                    let ev = super::screens::agent_feed::FeedEvent {
                        ts,
                        agent_id,
                        domain_id,
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

/// Try to parse agent_id / domain_id from the SSE data payload (expected to be JSON).
fn parse_feed_data(data: &str) -> (Option<String>, Option<String>, String) {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
        let agent_id = v
            .get("agent_id")
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .or_else(|| v.get("agent").and_then(|x| x.as_str()).map(str::to_string));
        let domain_id = v
            .get("domain_id")
            .and_then(|x| x.as_str())
            .map(str::to_string);
        let summary = v
            .get("message")
            .or_else(|| v.get("data"))
            .or_else(|| v.get("summary"))
            .and_then(|x| x.as_str())
            .unwrap_or(data)
            .to_string();
        (agent_id, domain_id, summary)
    } else {
        (None, None, data.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::data::{DataClient, DomainSummary, FixtureDataClient};
    use std::sync::Arc;

    #[tokio::test]
    async fn pool_delivers_ping_result() {
        let pool = WorkPool::new();
        let client: Arc<dyn DataClient> = Arc::new(FixtureDataClient::default());
        let job_id = next_job_id();
        pool.dispatch(client, WorkRequest::Ping { job_id });
        let result = pool
            .result_rx
            .recv_timeout(std::time::Duration::from_secs(5));
        assert!(result.is_ok(), "no result arrived");
        match result.unwrap() {
            WorkResult::Pinged {
                job_id: jid, ok, ..
            } => {
                assert_eq!(jid, job_id);
                assert!(ok);
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[tokio::test]
    async fn pool_delivers_domains_result() {
        let pool = WorkPool::new();
        let client: Arc<dyn DataClient> = Arc::new(FixtureDataClient {
            domains: vec![DomainSummary {
                id: "m1".into(),
                name: "Test Domain".into(),
                status: "active".into(),
            }],
        });
        let job_id = next_job_id();
        pool.dispatch(client, WorkRequest::ListDomains { job_id });
        let result = pool
            .result_rx
            .recv_timeout(std::time::Duration::from_secs(5));
        assert!(result.is_ok());
        match result.unwrap() {
            WorkResult::DomainsListed { domains, error, .. } => {
                assert!(error.is_none());
                assert_eq!(domains.len(), 1);
                assert_eq!(domains[0].name, "Test Domain");
            }
            _ => panic!("unexpected variant"),
        }
    }
}
