/// Per-agent task claim → inject → forward loop.
///
/// Each supervised agent runs one of these concurrently.  The loop:
///   1. Polls the backend for ready tasks in all missions of the agent's domain
///   2. Claims the highest-priority eligible task
///   3. Injects the task into the agent runtime
///   4. Forwards progress events to the backend in real time
///   5. Heartbeats the lease every HEARTBEAT_INTERVAL
///   6. Marks the task complete or failed when the progress stream closes
///
/// A parallel message relay loop polls inbound messages and delivers them to
/// the runtime via `AgentRuntime::signal()`.
///
/// A parallel notify WS loop connects to `/agents/{id}/notify` and wakes
/// the main loop immediately when a `task_available` push arrives from the
/// backend, reducing idle latency without changing error-path behavior.
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use edgeplaned_core::client::BackendClient;
use edgeplaned_core::types::{AgentHandle, AgentSignal, PendingPeerMessage, TaskSpec};
use edgeplaned_work::task::TaskError;
use edgeplaned_work::watchdog::{ConnectivityState, OfflinePolicy};
use edgeplaned_work::{claim, task};
use futures::StreamExt;

use crate::attach_registry::AttachRegistry;

const POLL_INTERVAL_MIN: Duration = Duration::from_secs(5);
const POLL_INTERVAL_MAX: Duration = Duration::from_secs(30);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const MESSAGE_POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Run the task loop for a single agent.  Never returns under normal operation.
///
/// Spawns a parallel message relay loop so inbound peer messages are delivered
/// to the runtime via `signal()` even while a task is being executed.
pub async fn run_for_agent(
    agent: Arc<tokio::sync::Mutex<AgentHandle>>,
    runtime: Arc<edgeplaned_core::agent_runtime::DynAgentRuntime>,
    client: Arc<BackendClient>,
    domain_id: String,
    agent_id: String,
    watchdog: Arc<edgeplaned_work::watchdog::Watchdog>,
) {
    // Buffer for peer messages that arrive while no task is running. The
    // relay appends here when there's no live PTY/ACP session to deliver to;
    // the main loop drains it into each TaskSpec before inject.
    let pending_msgs: Arc<tokio::sync::Mutex<Vec<PendingPeerMessage>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    // Spawn the message relay as a detached background task.
    {
        let relay_agent = Arc::clone(&agent);
        let relay_runtime = Arc::clone(&runtime);
        let relay_client = Arc::clone(&client);
        let relay_agent_id = agent_id.clone();
        let relay_buffer = Arc::clone(&pending_msgs);
        tokio::spawn(async move {
            run_message_relay(
                relay_agent,
                relay_runtime,
                relay_client,
                relay_agent_id,
                None,
                Some(relay_buffer),
            )
            .await;
        });
    }

    // Spawn a WebSocket notify listener that wakes the main loop on task_available push.
    let (wake_tx, mut wake_rx) = tokio::sync::watch::channel(false);
    {
        let notify_client = Arc::clone(&client);
        let notify_agent_id = agent_id.clone();
        tokio::spawn(async move {
            run_notify_ws(notify_client, notify_agent_id, wake_tx).await;
        });
    }

    // Track the last claimed task so we can fail it if we go offline mid-run.
    let mut current_task_id: Option<String> = None;
    let mut current_lease_id: Option<String> = None;
    // Adaptive backoff: doubles on idle/error iterations, resets on claim.
    let mut poll_interval = POLL_INTERVAL_MIN;

    loop {
        // Enforce offline policy before doing any work.
        let connectivity = *watchdog.state_rx.borrow();
        match (watchdog.policy(), connectivity) {
            // Strict: if offline, fail any in-flight task and stop claiming.
            (OfflinePolicy::Strict, ConnectivityState::Offline { .. }) => {
                if let Some(tid) = current_task_id.take() {
                    tracing::warn!(
                        "Watchdog strict offline: failing in-flight task {tid} for agent {agent_id}"
                    );
                    // Best-effort: can't reach backend, but record locally.
                    let _ = task::fail_task(
                        &client,
                        &tid,
                        current_lease_id.as_deref(),
                        "watchdog: offline (strict)",
                    )
                    .await;
                    current_lease_id = None;
                }
                tracing::warn!("Watchdog strict offline: pausing task loop for agent {agent_id}");
                tokio::time::sleep(poll_interval).await;
                poll_interval = (poll_interval * 2).min(POLL_INTERVAL_MAX);
                continue;
            }
            // SafeReadonly: pause claiming but don't actively fail tasks.
            (OfflinePolicy::SafeReadonly, ConnectivityState::Offline { .. }) => {
                tracing::info!(
                    "Watchdog safe-readonly offline: suspending claims for agent {agent_id}"
                );
                tokio::time::sleep(poll_interval).await;
                poll_interval = (poll_interval * 2).min(POLL_INTERVAL_MAX);
                continue;
            }
            // Autonomous: continue until the TTL is exceeded, then act like Strict.
            (OfflinePolicy::Autonomous { max_ttl_secs }, ConnectivityState::Offline { since }) => {
                let elapsed = (chrono::Utc::now() - since).num_seconds().unsigned_abs();
                if elapsed > max_ttl_secs {
                    tracing::warn!(
                        "Watchdog autonomous TTL {max_ttl_secs}s exceeded for agent {agent_id}: stopping"
                    );
                    if let Some(tid) = current_task_id.take() {
                        let _ = task::fail_task(
                            &client,
                            &tid,
                            current_lease_id.as_deref(),
                            "watchdog: autonomous TTL exceeded",
                        )
                        .await;
                        current_lease_id = None;
                    }
                    tokio::time::sleep(poll_interval).await;
                    poll_interval = (poll_interval * 2).min(POLL_INTERVAL_MAX);
                    continue;
                }
                // Within TTL — fall through and keep running.
                tracing::debug!(
                    "Watchdog autonomous offline: {elapsed}s/{max_ttl_secs}s elapsed, continuing for {agent_id}"
                );
            }
            _ => {} // Connected or Degraded — proceed normally.
        }

        // Heartbeat the agent itself.
        if let Err(e) = client
            .raw_post(
                &format!("/agents/{agent_id}/heartbeat"),
                &serde_json::json!({}),
            )
            .await
        {
            tracing::warn!("Agent heartbeat failed: {e}");
            watchdog.record_heartbeat_failure();
            tokio::time::sleep(poll_interval).await;
            poll_interval = (poll_interval * 2).min(POLL_INTERVAL_MAX);
            continue;
        }
        watchdog.record_heartbeat_success();

        // Get missions for this domain.
        let missions = match get_domain_missions(&client, &domain_id).await {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!("Could not list missions for domain {domain_id}: {e}");
                tokio::time::sleep(poll_interval).await;
                poll_interval = (poll_interval * 2).min(POLL_INTERVAL_MAX);
                continue;
            }
        };

        // Try to claim a task from any mission.
        let caps = runtime.capabilities().to_vec();
        let mut claimed: Option<claim::ClaimOutcome> = None;
        for mission_id in &missions {
            match claim::try_claim_one(&client, mission_id, &caps).await {
                Ok(Some(outcome)) => {
                    claimed = Some(outcome);
                    break;
                }
                Ok(None) => {}
                Err(e) => tracing::debug!("Claim attempt error in {mission_id}: {e}"),
            }
        }

        let Some(outcome) = claimed else {
            // Sleep until the interval expires OR the backend pushes a task notification.
            tokio::select! {
                _ = tokio::time::sleep(poll_interval) => {
                    poll_interval = (poll_interval * 2).min(POLL_INTERVAL_MAX);
                }
                _ = wake_rx.changed() => {
                    let _ = wake_rx.borrow_and_update();
                    poll_interval = POLL_INTERVAL_MIN;
                    tracing::debug!("task_available notification received for {agent_id}, polling immediately");
                }
            }
            continue;
        };

        let task_record = outcome.task;
        let lease_id = outcome.claim_lease_id;

        tracing::info!(
            "Agent {agent_id} claimed task {} ({}) lease={:?}",
            task_record.id,
            task_record.title,
            lease_id,
        );
        // Reset backoff now that we have work.
        poll_interval = POLL_INTERVAL_MIN;

        // Update agent status to busy.
        let _ = client
            .raw_post(
                &format!("/agents/{agent_id}/status?status=busy"),
                &serde_json::json!({}),
            )
            .await;

        // Fetch agent profile and domain roster for context injection.
        let agent_profile = client
            .get_agent(&agent_id)
            .await
            .ok()
            .and_then(|v| v.get("profile").cloned())
            .filter(|v| !v.is_null());

        let domain_roster = client
            .get_domain_roster(&domain_id)
            .await
            .unwrap_or_default()
            .into_iter()
            // Exclude this agent from the roster it sees (it knows itself already).
            .filter(|entry| entry.get("id").and_then(|v| v.as_str()) != Some(&agent_id))
            .collect::<Vec<_>>();

        // Fetch terminal summaries from upstream dependencies, if any. This
        // gets spliced into the prompt as `[DEPENDENCY RESULTS]` by the
        // shared `build_prompt`. Best-effort — failures don't block inject.
        let dependency_results = if task_record.depends_on.is_empty() {
            Vec::new()
        } else {
            task::fetch_dependency_results(&client, &task_record.depends_on).await
        };

        // Drain any buffered peer messages — they get spliced into the
        // prompt as `[PENDING MESSAGES]` so single-shot runtimes can see
        // signals that arrived while no task was running.
        let pending_messages: Vec<PendingPeerMessage> = {
            let mut buf = pending_msgs.lock().await;
            std::mem::take(&mut *buf)
        };

        // Build the TaskSpec.
        let task_spec = TaskSpec {
            id: task_record.id.clone(),
            mission_id: task_record.mission_id.clone(),
            domain_id: domain_id.clone(),
            title: task_record.title.clone(),
            description: task_record.description.clone(),
            input_json: "{}".into(),
            required_capabilities: task_record.required_capabilities.clone(),
            produces: task_record.produces.clone(),
            consumes: task_record.consumes.clone(),
            agent_profile,
            domain_roster,
            dependency_results,
            pending_messages,
        };

        // Inject and stream progress.
        let handle = agent.lock().await;
        let stream = match runtime.inject_task(&handle, &task_spec).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("inject_task failed: {e}");
                let _ = task::fail_task(
                    &client,
                    &task_record.id,
                    lease_id.as_deref(),
                    &e.to_string(),
                )
                .await;
                drop(handle);
                set_agent_idle(&client, &agent_id).await;
                continue;
            }
        };
        drop(handle);

        // Forward progress events with lease heartbeat.
        let result = stream_and_heartbeat(
            stream,
            &client,
            &task_record.id,
            &agent_id,
            lease_id.as_deref(),
        )
        .await;

        match result {
            Ok(success) => {
                if success {
                    match task::complete_task(&client, &task_record.id, lease_id.as_deref(), None)
                        .await
                    {
                        Err(TaskError::LeaseMismatch) => {
                            tracing::warn!(
                                "lease mismatch or stolen, abandoning task {}",
                                task_record.id
                            );
                        }
                        Err(TaskError::Other(e)) => {
                            tracing::error!("complete_task error: {e}");
                        }
                        Ok(()) => {}
                    }
                } else {
                    match task::fail_task(
                        &client,
                        &task_record.id,
                        lease_id.as_deref(),
                        "agent reported failure",
                    )
                    .await
                    {
                        Err(TaskError::LeaseMismatch) => {
                            tracing::warn!(
                                "lease mismatch or stolen, abandoning task {}",
                                task_record.id
                            );
                        }
                        Err(TaskError::Other(e)) => {
                            tracing::error!("fail_task error: {e}");
                        }
                        Ok(()) => {}
                    }
                }
            }
            Err(e) => {
                tracing::error!("stream_and_heartbeat error: {e}");
                let _ = task::fail_task(
                    &client,
                    &task_record.id,
                    lease_id.as_deref(),
                    &e.to_string(),
                )
                .await;
            }
        }

        set_agent_idle(&client, &agent_id).await;
    }
}

/// Forward a progress stream to the backend, heartbeating the lease in parallel.
///
/// Returns `Err` on transport errors.  A 409 heartbeat response causes a
/// warning log and terminates the stream loop (the lease was stolen).
async fn stream_and_heartbeat(
    mut stream: futures::stream::BoxStream<'static, edgeplaned_core::progress::ProgressEvent>,
    client: &BackendClient,
    task_id: &str,
    _agent_id: &str,
    claim_lease_id: Option<&str>,
) -> Result<bool> {
    let mut last_heartbeat = std::time::Instant::now();
    let mut success = true;

    while let Some(event) = stream.next().await {
        // Check if this is a final error event.
        if event.event_type == edgeplaned_core::progress::ProgressEventType::Error {
            success = false;
        }

        if let Err(e) = task::post_progress(client, task_id, &event).await {
            tracing::warn!("Progress post failed: {e}");
        }

        // Heartbeat if overdue.
        if last_heartbeat.elapsed() >= HEARTBEAT_INTERVAL {
            match task::heartbeat_task(client, task_id, claim_lease_id).await {
                Ok(()) => {}
                Err(TaskError::LeaseMismatch) => {
                    tracing::warn!("lease mismatch or stolen, abandoning task {task_id}");
                    return Ok(false);
                }
                Err(TaskError::Other(e)) => {
                    tracing::warn!("Lease heartbeat failed: {e}");
                }
            }
            last_heartbeat = std::time::Instant::now();
        }
    }

    Ok(success)
}

/// Get all mission ids for a domain.
async fn get_domain_missions(client: &BackendClient, domain_id: &str) -> Result<Vec<String>> {
    let resp: serde_json::Value = client.get(&format!("/domains/{domain_id}/m")).await?;

    // Backend returns an array of mission objects with an "id" field.
    let ids = resp
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|k| k.get("id").and_then(|v| v.as_str()).map(String::from))
        .collect();

    Ok(ids)
}

async fn set_agent_idle(client: &BackendClient, agent_id: &str) {
    let _ = client
        .raw_post(
            &format!("/agents/{agent_id}/status?status=idle"),
            &serde_json::json!({}),
        )
        .await;
}

/// Poll inbound messages for this agent and deliver them.
///
/// For task-mode agents (`registry: None`), messages are delivered via
/// `AgentRuntime::signal()` — the existing path. For persistent-mode agents
/// (`registry: Some(...)`), if the session supervisor has registered a signal
/// channel, the message is routed there; the runtime's `signal()` is bypassed.
/// If the persistent agent isn't registered yet (e.g. PTY restarting), we
/// silently drop the message — the supervisor will re-register on relaunch.
pub async fn run_message_relay(
    agent: Arc<tokio::sync::Mutex<AgentHandle>>,
    runtime: Arc<edgeplaned_core::agent_runtime::DynAgentRuntime>,
    client: Arc<BackendClient>,
    agent_id: String,
    registry: Option<Arc<AttachRegistry>>,
    pending_buffer: Option<Arc<tokio::sync::Mutex<Vec<PendingPeerMessage>>>>,
) {
    // We poll the agent-scoped message inbox: GET /agents/{id}/messages
    // which returns messages addressed to this agent + domain broadcasts.
    //
    // `last_id` is an in-memory cursor; it resets to 0 on every edgeplaned restart.
    // To avoid re-delivering the full message history on each startup, the
    // first poll drains the cursor to the current high-water mark without
    // routing any messages to the session. Only messages that arrive *after*
    // edgeplaned starts are delivered.
    let mut last_id: i64 = 0;
    let mut startup_drain = true;

    loop {
        tokio::time::sleep(MESSAGE_POLL_INTERVAL).await;

        let path = format!("/agents/{agent_id}/messages?since_id={last_id}");
        let msgs: Vec<serde_json::Value> = match client.get(&path).await {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!("Message poll failed for agent {agent_id}: {e}");
                continue;
            }
        };

        if startup_drain {
            // Advance cursor past any pre-existing messages without delivering them.
            let high = msgs
                .iter()
                .filter_map(|m| m.get("id").and_then(|v| v.as_i64()))
                .max();
            if let Some(max) = high {
                tracing::debug!(
                    "Message relay startup drain for {agent_id}: skipping {} existing message(s), cursor → {max}",
                    msgs.len()
                );
                last_id = max;
            }
            startup_drain = false;
            continue;
        }

        for msg in msgs {
            // Track the highest seen id so we don't re-deliver.
            if let Some(id) = msg.get("id").and_then(|v| v.as_i64())
                && id > last_id
            {
                last_id = id;
            }

            // from_agent_id may be an integer (agent table id) or a string public_id.
            let from_agent_id = msg
                .get("from_agent_id")
                .map(|v| {
                    v.as_str()
                        .map(String::from)
                        .unwrap_or_else(|| v.to_string())
                })
                .unwrap_or_else(|| "unknown".to_string());
            // channel: native peer messages use "channel"; edgeplane agent signal messages use
            // message_type ("signal", "command") with no channel field.
            let channel = msg
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| {
                    msg.get("message_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("coordination")
                })
                .to_string();
            // body: native peer messages use body_json; edgeplane agent signal stores text
            // in the "content" field. Fall back gracefully so signal content is not
            // silently dropped.
            let body: serde_json::Value = msg
                .get("body_json")
                .and_then(|v| v.as_str())
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_else(|| {
                    if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
                        serde_json::json!({"text": content})
                    } else {
                        msg.get("body_json")
                            .cloned()
                            .unwrap_or(serde_json::json!({}))
                    }
                });

            tracing::info!(
                "Agent {agent_id} received message from {from_agent_id} on channel {channel}"
            );

            let signal = AgentSignal::PeerMessage {
                from_agent_id: from_agent_id.clone(),
                channel: channel.clone(),
                body: body.clone(),
            };

            // Persistent agents: route to the registered session supervisor
            // for live PTY/ACP delivery.
            let mut delivered = false;
            if let Some(ref reg) = registry
                && let Some(endpoints) = reg.get(&agent_id).await
            {
                if let Err(e) = endpoints.signal_tx().send(signal.clone()).await {
                    tracing::debug!(
                        "Session supervisor not ready for {agent_id}, falling back: {e}"
                    );
                } else {
                    delivered = true;
                }
            }

            if !delivered {
                // Task-mode agents: buffer for the next task inject. Single-shot
                // runtimes have no stdin to inject into once spawned, so
                // runtime.signal() would drop the message. The buffer is drained
                // by the main task_loop into TaskSpec.pending_messages before
                // the next inject_task call.
                if let Some(ref buf) = pending_buffer {
                    let mut guard = buf.lock().await;
                    guard.push(PendingPeerMessage {
                        from_agent_id,
                        channel,
                        body,
                        received_at: chrono::Utc::now().to_rfc3339(),
                    });
                    tracing::debug!(
                        "Buffered peer message for {agent_id} (buffer size now {})",
                        guard.len()
                    );
                } else {
                    // No buffer (persistent agent path with supervisor not yet
                    // attached) — fall back to runtime.signal(), which is a
                    // no-op for task runtimes but valid for ones that handle it.
                    let handle = agent.lock().await;
                    if let Err(e) = runtime.signal(&handle, signal).await {
                        tracing::warn!("signal() delivery failed for agent {agent_id}: {e}");
                    }
                }
            }
        }
    }
}

/// Message relay for persistent agents that expose a local webhook.
///
/// Polls the controlplane inbox exactly like `run_message_relay` but delivers
/// each message by POSTing `{"text": "<content>"}` to `webhook_url` instead of
/// routing through an ACP process. The webhook server (started by
/// `edgeplane channel claude webhook --listen-port <N>`) translates the POST into a
/// `notifications/claude/channel` MCP notification, which claude delivers as a
/// `session/prompt`. No competing claude process is spawned.
pub async fn run_webhook_relay(client: Arc<BackendClient>, agent_id: String, webhook_url: String) {
    let http = reqwest::Client::new();
    let mut last_id: i64 = 0;
    let mut startup_drain = true;

    loop {
        tokio::time::sleep(MESSAGE_POLL_INTERVAL).await;

        let path = format!("/agents/{agent_id}/messages?since_id={last_id}");
        let msgs: Vec<serde_json::Value> = match client.get(&path).await {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!("Webhook relay poll failed for {agent_id}: {e}");
                continue;
            }
        };

        if startup_drain {
            let high = msgs
                .iter()
                .filter_map(|m| m.get("id").and_then(|v| v.as_i64()))
                .max();
            if let Some(max) = high {
                tracing::debug!(
                    "Webhook relay startup drain for {agent_id}: skipping {} messages, cursor → {max}",
                    msgs.len()
                );
                last_id = max;
            }
            startup_drain = false;
            continue;
        }

        for msg in &msgs {
            if let Some(id) = msg.get("id").and_then(|v| v.as_i64())
                && id > last_id
            {
                last_id = id;
            }

            let text = msg
                .get("content")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| {
                    msg.get("body_json")
                        .and_then(|v| v.as_str())
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                        .and_then(|b| b.get("text").and_then(|t| t.as_str()).map(String::from))
                });

            let Some(text) = text else {
                tracing::debug!("Webhook relay: no deliverable content in message for {agent_id}");
                continue;
            };

            let from = msg
                .get("from_agent_id")
                .map(|v| {
                    v.as_str()
                        .map(String::from)
                        .unwrap_or_else(|| v.to_string())
                })
                .unwrap_or_else(|| "unknown".to_string());
            let channel = msg
                .get("channel")
                .and_then(|v| v.as_str())
                .or_else(|| msg.get("message_type").and_then(|v| v.as_str()))
                .unwrap_or("coordination");

            tracing::info!(
                "Webhook relay: delivering message for {agent_id} from {from} on {channel}"
            );

            let payload = serde_json::json!({"text": text});
            match http.post(&webhook_url).json(&payload).send().await {
                Ok(resp) if resp.status().is_success() => {}
                Ok(resp) => {
                    tracing::warn!("Webhook delivery returned {} for {agent_id}", resp.status())
                }
                Err(e) => tracing::warn!("Webhook delivery failed for {agent_id}: {e}"),
            }
        }
    }
}

/// Connect to `/agents/{id}/notify` via WebSocket and signal `wake_tx`
/// whenever a `task_available` push arrives.  Reconnects with exponential
/// backoff (2s → 60s) on disconnect or error.  Runs forever — drop the task
/// to stop it.
///
/// Takes `Arc<BackendClient>` rather than a token snapshot so that each
/// reconnect attempt reads the live token via `client.current_token()`.
/// This mirrors the pattern in `reconcile::watch_assignments_ws` and ensures
/// that a node-JWT rotation mid-lifetime does not leave the WS loop
/// authenticating with the now-revoked old credential.
async fn run_notify_ws(
    client: Arc<BackendClient>,
    agent_id: String,
    wake_tx: tokio::sync::watch::Sender<bool>,
) {
    use tokio_tungstenite::{connect_async, tungstenite};

    let ws_base = client
        .base_url
        .trim_end_matches('/')
        .replacen("http://", "ws://", 1)
        .replacen("https://", "wss://", 1);
    let url = format!("{ws_base}/agents/{agent_id}/notify");

    let mut backoff = Duration::from_secs(2);
    const MAX_BACKOFF: Duration = Duration::from_secs(60);

    loop {
        // Read the live token at each (re)connect so a rotation between
        // attempts uses the current credential rather than the one captured
        // at task-loop start.
        let token = client.current_token();
        let request =
            match tungstenite::client::IntoClientRequest::into_client_request(url.as_str()) {
                Ok(mut req) => {
                    req.headers_mut().insert(
                        "Authorization",
                        format!("Bearer {token}")
                            .parse()
                            .expect("valid header value"),
                    );
                    req
                }
                Err(e) => {
                    tracing::warn!("notify WS: failed to build request for {agent_id}: {e}");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                    continue;
                }
            };

        match connect_async(request).await {
            Ok((mut ws, _)) => {
                tracing::debug!("notify WS connected for agent {agent_id}");
                backoff = Duration::from_secs(2); // reset on successful connect
                loop {
                    use futures::StreamExt as _;
                    match ws.next().await {
                        Some(Ok(tungstenite::Message::Text(txt))) => {
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt)
                                && v.get("type").and_then(|t| t.as_str()) == Some("task_available")
                            {
                                let _ = wake_tx.send(true);
                            }
                        }
                        Some(Ok(_)) => {} // ping/binary frames ignored
                        Some(Err(e)) => {
                            tracing::debug!("notify WS error for {agent_id}: {e}");
                            break;
                        }
                        None => break, // server closed
                    }
                }
                tracing::debug!("notify WS disconnected for agent {agent_id}, reconnecting");
            }
            Err(e) => {
                tracing::debug!("notify WS connect failed for {agent_id}: {e}");
            }
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}
