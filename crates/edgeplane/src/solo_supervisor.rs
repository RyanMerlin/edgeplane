/// SoloSupervisor — run an agent runtime in the foreground while participating
/// in the mesh as a MeshAgent with supervision_mode=solo.
///
/// Does NOT require the edgeplaned daemon.  Talks directly to the backend HTTP API
/// using the same `EdgeplaneClient` that `edgeplane` already uses.
///
/// Two modes:
/// - `run_solo`: one-shot launch — enroll → launch → offline.
/// - `run_solo_work_loop`: persistent worker — enroll → loop claiming and
///   executing ready tasks via the domain WebSocket stream → offline on stop.
use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::client::EdgeplaneClient;

/// Enroll a MeshAgent, start a run record, execute `launch_fn`, then clean up.
///
/// `launch_fn` receives `(agent_id, run_id)` so it can inject `EP_MESH_AGENT_ID`
/// and `EP_RUN_ID` into the child process environment.  It must block until the
/// child exits and return the exit status.
pub async fn run_solo<F>(
    client: &EdgeplaneClient,
    domain_id: &str,
    runtime_kind: &str,
    profile_name: &str,
    launch_fn: F,
) -> Result<ExitStatus>
where
    F: FnOnce(&str, Option<&str>) -> Result<ExitStatus> + Send + 'static,
{
    // 1. Enroll the agent with supervision_mode=solo.
    let enroll_body = json!({
        "runtime_kind": runtime_kind,
        "capabilities": ["tool_use", "file_edit"],
        "labels": {},
        "supervision_mode": "solo",
        "profile": {"name": profile_name},
    });
    let enroll_path = format!("/work/domains/{domain_id}/agents/enroll");
    let enroll_resp = client
        .post_json(&enroll_path, &enroll_body)
        .await
        .context("failed to enroll mesh agent")?;
    let agent_id = enroll_resp
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if agent_id.is_empty() {
        anyhow::bail!("backend returned empty agent id during enroll");
    }
    eprintln!(
        "edgeplane: enrolled as mesh agent {} (domain {})",
        agent_id, domain_id
    );

    // 2. Create a run record (best-effort — doesn't abort if backend doesn't support it yet).
    let run_resp = client
        .post_json(
            "/runs",
            &json!({
                "mesh_agent_id": agent_id,
                "runtime_kind": runtime_kind,
            }),
        )
        .await
        .ok();
    let run_id: Option<String> = run_resp
        .as_ref()
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    // 3. Start background heartbeat thread.
    //    EdgeplaneClient is Clone so we hand a clone to the thread.
    let hb_client = client.clone();
    let hb_agent_id = agent_id.clone();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let hb_stop = Arc::clone(&stop_flag);

    let hb_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("heartbeat runtime");
        rt.block_on(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(25)).await;
                if hb_stop.load(Ordering::Relaxed) {
                    break;
                }
                let _ = hb_client
                    .post_json(
                        &format!("/work/agents/{hb_agent_id}/heartbeat"),
                        &json!({}),
                    )
                    .await;
            }
        });
    });

    // 4. Launch the runtime — blocks until the child exits.
    let status = launch_fn(&agent_id, run_id.as_deref())?;

    // 5. Stop heartbeat thread.
    stop_flag.store(true, Ordering::Relaxed);
    let _ = hb_thread.join();

    // 6. Mark agent offline.
    let _ = client
        .post_json(
            &format!("/work/agents/{agent_id}/status?status=offline"),
            &json!({}),
        )
        .await;

    // 7. Transition run to completed or failed.
    if let Some(ref rid) = run_id {
        let terminal = if status.success() { "completed" } else { "failed" };
        let _ = client
            .post_json(&format!("/runs/{rid}/{terminal}"), &json!({}))
            .await;
    }

    eprintln!("edgeplane: agent {} offline", agent_id);
    Ok(status)
}

/// Run a persistent work loop: enroll, claim ready tasks from the domain event
/// stream, execute `task_launch_fn` per task, then loop.
///
/// `task_launch_fn(agent_id, task_id, task_md_path)` must block until the runtime
/// exits and return its exit status.  Returning `Ok(success)` completes the task;
/// returning `Err(_)` or `Ok(failure)` fails it.
///
/// The loop exits on Ctrl-C.  The agent is marked offline before returning.
pub async fn run_solo_work_loop<F>(
    client: &EdgeplaneClient,
    domain_id: &str,
    runtime_kind: &str,
    profile_name: &str,
    task_launch_fn: F,
) -> Result<()>
where
    F: Fn(&str, &str, &std::path::Path) -> Result<ExitStatus> + Send + Clone + 'static,
{
    // ---- 1. Enroll ----
    let enroll_body = json!({
        "runtime_kind": runtime_kind,
        "capabilities": ["tool_use", "file_edit"],
        "labels": {},
        "supervision_mode": "solo",
        "profile": {"name": profile_name},
    });
    let enroll_resp = client
        .post_json(&format!("/work/domains/{domain_id}/agents/enroll"), &enroll_body)
        .await
        .context("failed to enroll mesh agent")?;
    let agent_id = enroll_resp
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if agent_id.is_empty() {
        anyhow::bail!("backend returned empty agent id during enroll");
    }
    eprintln!("edgeplane: work loop enrolled as agent {} (domain {})", agent_id, domain_id);

    // ---- 2. Heartbeat thread ----
    let hb_client = client.clone();
    let hb_agent_id = agent_id.clone();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let hb_stop = Arc::clone(&stop_flag);
    let hb_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("heartbeat runtime");
        rt.block_on(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(25)).await;
                if hb_stop.load(Ordering::Relaxed) {
                    break;
                }
                let _ = hb_client
                    .post_json(&format!("/work/agents/{hb_agent_id}/heartbeat"), &json!({}))
                    .await;
            }
        });
    });

    // ---- 3. Initial poll for already-ready tasks ----
    let mut ready_queue: VecDeque<String> = VecDeque::new();
    if let Ok(missions) = client.get_json(&format!("/domains/{domain_id}/m")).await {
        for k in missions.as_array().unwrap_or(&vec![]) {
            if let Some(kid) = k["id"].as_str() {
                if let Ok(tasks) = client
                    .get_json(&format!("/work/missions/{kid}/tasks?status=ready"))
                    .await
                {
                    for t in tasks.as_array().unwrap_or(&vec![]) {
                        if let Some(tid) = t["id"].as_str() {
                            ready_queue.push_back(tid.to_string());
                        }
                    }
                }
            }
        }
    }

    // ---- 4. Work loop ----
    let work_dir = std::env::temp_dir().join(format!("edgeplaned-{}", &agent_id[..8]));
    let _ = std::fs::create_dir_all(&work_dir);

    let result = work_loop_inner(
        client,
        domain_id,
        &agent_id,
        ready_queue,
        work_dir,
        task_launch_fn,
    )
    .await;

    // ---- 5. Cleanup ----
    stop_flag.store(true, Ordering::Relaxed);
    let _ = hb_thread.join();
    let _ = client
        .post_json(&format!("/work/agents/{agent_id}/status?status=offline"), &json!({}))
        .await;
    eprintln!("edgeplane: work loop agent {} offline", agent_id);

    result
}

async fn work_loop_inner<F>(
    client: &EdgeplaneClient,
    domain_id: &str,
    agent_id: &str,
    mut ready_queue: VecDeque<String>,
    work_dir: PathBuf,
    task_launch_fn: F,
) -> Result<()>
where
    F: Fn(&str, &str, &std::path::Path) -> Result<ExitStatus> + Send + Clone + 'static,
{
    let mut backoff = Duration::from_secs(1);

    'reconnect: loop {
        // Drain any queued ready tasks before connecting/reconnecting.
        if let Some(task_id) = ready_queue.pop_front() {
            attempt_claim_and_run(client, agent_id, &task_id, &work_dir, &task_launch_fn).await;
            // After executing, loop back to check queue or subscribe.
            continue 'reconnect;
        }

        // Connect WebSocket to domain stream.
        let mut url = client.ws_url(&format!("/work/domains/{domain_id}/stream"))?;
        if let Some(token) = client.token() {
            url.query_pairs_mut().append_pair("token", token);
        }

        match connect_async(url.as_str()).await {
            Ok((mut ws, _)) => {
                backoff = Duration::from_secs(1);
                eprintln!("edgeplane: subscribed to domain {} event stream", domain_id);

                while let Some(msg) = ws.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            if let Ok(event) = serde_json::from_str::<Value>(&text) {
                                if event["event"].as_str() == Some("task_ready") {
                                    if let Some(tid) = event["task_id"].as_str() {
                                        ready_queue.push_back(tid.to_string());
                                    }
                                }
                            }
                            // Process one task per iteration to stay responsive to the stream.
                            if let Some(task_id) = ready_queue.pop_front() {
                                attempt_claim_and_run(
                                    client, agent_id, &task_id, &work_dir, &task_launch_fn,
                                )
                                .await;
                            }
                        }
                        Ok(Message::Close(_)) | Err(_) => break,
                        _ => {}
                    }
                }

                eprintln!(
                    "edgeplane: domain stream disconnected — reconnecting in {}s…",
                    backoff.as_secs()
                );
            }
            Err(e) => {
                eprintln!(
                    "edgeplane: domain stream connect failed: {e} — retrying in {}s…",
                    backoff.as_secs()
                );
            }
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

/// Try to claim a task and, if successful, execute it.
async fn attempt_claim_and_run<F>(
    client: &EdgeplaneClient,
    agent_id: &str,
    task_id: &str,
    work_dir: &std::path::Path,
    task_launch_fn: &F,
) where
    F: Fn(&str, &str, &std::path::Path) -> Result<ExitStatus> + Send + Clone + 'static,
{
    // Claim the task.
    let claim_result = client
        .post_json(&format!("/work/tasks/{task_id}/claim"), &json!({}))
        .await;

    let claim = match claim_result {
        Ok(v) => v,
        Err(e) => {
            // 423 means another agent already took it — not an error.
            let msg = e.to_string();
            if !msg.contains("423") && !msg.contains("409") {
                eprintln!("edgeplane: claim {task_id} failed: {e}");
            }
            return;
        }
    };

    let claim_lease_id = claim["claim_lease_id"].as_str().unwrap_or("").to_string();
    let task_title = claim["title"].as_str().unwrap_or(task_id).to_string();
    let task_owner = claim["created_by_subject"].as_str().unwrap_or("").to_string();
    let depends_on: Vec<String> = claim["depends_on"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();

    eprintln!("edgeplane: claimed task {task_id} — {task_title}");

    // Write TASK.md.
    let task_md_path = work_dir.join(format!("{task_id}.md"));
    let fm = crate::task_md::TaskFrontMatter {
        id: task_id.to_string(),
        title: task_title.clone(),
        description: claim["description"].as_str().unwrap_or("").to_string(),
        depends_on,
        owner: task_owner,
        status: "claimed".to_string(),
    };
    if let Err(e) = crate::task_md::write_task_md(&task_md_path, &fm) {
        eprintln!("edgeplane: failed to write TASK.md for {task_id}: {e}");
    }

    // Launch the runtime — blocking, so run on a dedicated thread.
    let agent_id_owned = agent_id.to_string();
    let task_id_owned = task_id.to_string();
    let task_md_path_owned = task_md_path.clone();
    let launch = task_launch_fn.clone();

    let exit_result = tokio::task::spawn_blocking(move || {
        launch(&agent_id_owned, &task_id_owned, &task_md_path_owned)
    })
    .await;

    // Complete or fail based on the exit result.
    let mut complete_body = json!({ "claim_lease_id": claim_lease_id });

    match exit_result {
        Ok(Ok(status)) if status.success() => {
            eprintln!("edgeplane: task {task_id} finished (exit 0)");
            let _ = client
                .post_json(&format!("/work/tasks/{task_id}/complete"), &complete_body)
                .await;
        }
        Ok(Ok(status)) => {
            let code = status.code().unwrap_or(-1);
            eprintln!("edgeplane: task {task_id} failed (exit {code})");
            complete_body["error"] = json!(format!("runtime exited {code}"));
            let _ = client
                .post_json(&format!("/work/tasks/{task_id}/fail"), &complete_body)
                .await;
        }
        Ok(Err(e)) => {
            eprintln!("edgeplane: task {task_id} launch error: {e}");
            complete_body["error"] = json!(e.to_string());
            let _ = client
                .post_json(&format!("/work/tasks/{task_id}/fail"), &complete_body)
                .await;
        }
        Err(e) => {
            eprintln!("edgeplane: task {task_id} spawn_blocking panic: {e}");
            complete_body["error"] = json!("spawn_blocking failed");
            let _ = client
                .post_json(&format!("/work/tasks/{task_id}/fail"), &complete_body)
                .await;
        }
    }

    // Clean up TASK.md.
    let _ = std::fs::remove_file(&task_md_path);
}
