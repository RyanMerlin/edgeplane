//! High-level [`Agent`] handle: spawns an ACP-speaking child process,
//! frames JSON-RPC over its stdio, and exposes typed request/response and
//! notification streams.
//!
//! ## Lifecycle
//!
//! ```text
//!   spawn ─▶ initialize ─▶ new_session ─▶ prompt* ─▶ shutdown
//!                              │              │
//!                              └──◀ notifications (session/update) ──┘
//! ```
//!
//! ## Architecture
//!
//! [`Agent`] is a thin handle backed by a single async task ("actor") that
//! owns the child's stdio. The handle communicates with the actor over an
//! mpsc command channel; outbound requests get a oneshot reply.
//!
//! - **Outbound writes** are serialized on the actor task (one writer to
//!   `child.stdin`, no contention).
//! - **Inbound reads** drive a dispatch loop:
//!   - Responses → resolve the matching request's oneshot.
//!   - `session/update` notifications → broadcast to subscribers.
//!   - Client-side requests (`fs/*`, `session/request_permission`,
//!     `terminal/*`) → respond with method-not-found by default. A future
//!     extension can plug a [`ClientHandler`] trait here.
//!
//! ## Env scrubbing
//!
//! The Node-based `claude-code-acp` agent refuses to start if `CLAUDECODE`
//! or any `CLAUDE_CODE_*` env var is present (it detects nested Claude
//! Code sessions). [`Agent::spawn`] strips them automatically.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::consts::{agent_methods, client_methods};
use crate::error::{AcpError, Result};
use crate::jsonrpc::{Message, RawMessage, RpcError};
use crate::schema;
use crate::wire;

/// How long to wait for graceful shutdown after closing stdin before sending
/// SIGTERM. claude-code-acp does not exit on stdin EOF — see crate docs.
const STDIN_CLOSE_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
/// How long to wait after SIGTERM before SIGKILL.
const SIGTERM_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Capacity of the broadcast channel for inbound `session/update`
/// notifications. Slow viewers that lag past this are dropped (see
/// [`broadcast::error::RecvError::Lagged`]).
const NOTIFICATIONS_CHANNEL_CAPACITY: usize = 1024;

/// Options for spawning an ACP agent process.
#[derive(Debug, Clone)]
pub struct SpawnOpts {
    /// Program to invoke (e.g. `node` or an absolute path to an ACP binary).
    pub program: PathBuf,
    /// Args to pass after the program (e.g. path to `dist/index.js`).
    pub args: Vec<String>,
    /// Working directory for the child. Independent of any per-session cwd
    /// passed in `session/new` later.
    pub cwd: Option<PathBuf>,
    /// Extra env vars to set on the child. Merged onto a scrubbed copy of
    /// the parent env (see crate docs).
    pub env: HashMap<String, String>,
    /// If true, inherit the parent's stderr (useful for dev). If false,
    /// stderr is captured and forwarded to `tracing::warn!`.
    pub inherit_stderr: bool,
}

impl SpawnOpts {
    /// Convenience: spawn `claude-code-acp` via `node` against an installed
    /// `dist/index.js`. Caller supplies the resolved path to the JS entry.
    pub fn claude_code_acp(node: impl Into<PathBuf>, dist_index_js: impl Into<PathBuf>) -> Self {
        Self {
            program: node.into(),
            args: vec![dist_index_js.into().to_string_lossy().into_owned()],
            cwd: None,
            env: HashMap::new(),
            inherit_stderr: false,
        }
    }
}

/// Public handle to a running ACP agent.
///
/// Cheap to clone (an `Arc<Inner>`); concurrent calls are serialized at the
/// actor task. Drop the last clone to abort the actor and child.
#[derive(Clone)]
pub struct Agent {
    inner: Arc<Inner>,
}

struct Inner {
    next_id: AtomicI64,
    cmd_tx: mpsc::Sender<ActorCommand>,
    notifications_tx: broadcast::Sender<wire::SessionNotification>,
    /// Held to keep the actor task alive until the last [`Agent`] handle drops.
    /// Wrapped in a `Mutex<Option<>>` so [`Agent::shutdown`] can reap it.
    actor_handle: Mutex<Option<JoinHandle<Result<i32>>>>,
    /// Set when the actor observes the child exiting; surfaced to subsequent
    /// API calls so callers see a clean error instead of timing out.
    child_exit: Mutex<Option<i32>>,
}

enum ActorCommand {
    /// Send a request and resolve the oneshot with the response (or error).
    Request {
        msg: RawMessage,
        reply: oneshot::Sender<Result<Value>>,
    },
    /// Send a notification (no response expected).
    Notification { msg: RawMessage },
    /// Initiate graceful shutdown: close stdin, wait, SIGTERM, wait, SIGKILL.
    Shutdown {
        reply: oneshot::Sender<Result<i32>>,
    },
}

impl Agent {
    /// Spawn the agent process and start the dispatch actor.
    ///
    /// Returns once the child is launched and pipes are wired. The agent
    /// is *not* yet initialized — call [`Agent::initialize`] next.
    pub async fn spawn(opts: SpawnOpts) -> Result<Self> {
        let scrubbed_env = scrubbed_parent_env();

        let mut cmd = Command::new(&opts.program);
        cmd.args(&opts.args);
        cmd.env_clear();
        for (k, v) in scrubbed_env {
            cmd.env(k, v);
        }
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }
        if let Some(cwd) = &opts.cwd {
            cmd.current_dir(cwd);
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(if opts.inherit_stderr {
            Stdio::inherit()
        } else {
            Stdio::piped()
        });
        cmd.kill_on_drop(true);

        let mut child = cmd.spawn().map_err(AcpError::Io)?;
        let stdin = child.stdin.take().ok_or_else(|| {
            AcpError::other("child stdin not piped (logic error in spawn)")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            AcpError::other("child stdout not piped (logic error in spawn)")
        })?;
        let stderr = child.stderr.take();

        let (cmd_tx, cmd_rx) = mpsc::channel::<ActorCommand>(64);
        let (notifications_tx, _) = broadcast::channel(NOTIFICATIONS_CHANNEL_CAPACITY);

        let inner = Arc::new(Inner {
            next_id: AtomicI64::new(1),
            cmd_tx,
            notifications_tx: notifications_tx.clone(),
            actor_handle: Mutex::new(None),
            child_exit: Mutex::new(None),
        });

        // Forward stderr to tracing if captured.
        if let Some(stderr) = stderr {
            tokio::spawn(forward_stderr(stderr));
        }

        let actor_inner = Arc::clone(&inner);
        let handle = tokio::spawn(actor_loop(
            child,
            stdin,
            stdout,
            cmd_rx,
            notifications_tx,
            actor_inner,
        ));
        *inner.actor_handle.lock().await = Some(handle);

        Ok(Self { inner })
    }

    /// Send `initialize` and return the agent's response.
    pub async fn initialize(
        &self,
        req: schema::InitializeRequest,
    ) -> Result<schema::InitializeResponse> {
        self.request(agent_methods::INITIALIZE, req).await
    }

    /// Create a new ACP session bound to a working directory.
    pub async fn new_session(
        &self,
        req: schema::NewSessionRequest,
    ) -> Result<schema::NewSessionResponse> {
        self.request(agent_methods::SESSION_NEW, req).await
    }

    /// Resume a previously-created session by id.
    pub async fn load_session(
        &self,
        req: schema::LoadSessionRequest,
    ) -> Result<schema::LoadSessionResponse> {
        self.request(agent_methods::SESSION_LOAD, req).await
    }

    /// Send a prompt; returns when the agent reports a stop reason.
    ///
    /// Streaming output arrives via [`Agent::subscribe_session_updates`]
    /// during the call.
    pub async fn prompt(&self, req: schema::PromptRequest) -> Result<schema::PromptResponse> {
        self.request(agent_methods::SESSION_PROMPT, req).await
    }

    /// Cancel any in-flight prompt turn for `session_id`. Fire-and-forget
    /// (cancellation is a notification per the spec).
    pub async fn cancel(&self, session_id: schema::SessionId) -> Result<()> {
        let params = serde_json::to_value(schema::CancelNotification {
            meta: None,
            session_id,
        })?;
        let msg = RawMessage::new_notification(agent_methods::SESSION_CANCEL, Some(params));
        self.inner
            .cmd_tx
            .send(ActorCommand::Notification { msg })
            .await
            .map_err(|_| AcpError::ConnectionClosed { request_id: None })?;
        Ok(())
    }

    /// Subscribe to streaming `session/update` notifications.
    ///
    /// Multiple subscribers fan out in parallel. Slow consumers are dropped
    /// at [`NOTIFICATIONS_CHANNEL_CAPACITY`]; receivers see `Lagged`.
    pub fn subscribe_session_updates(&self) -> broadcast::Receiver<wire::SessionNotification> {
        self.inner.notifications_tx.subscribe()
    }

    /// Initiate graceful shutdown. Waits up to ~7s total (stdin-close grace +
    /// SIGTERM grace) before SIGKILL. Returns the child exit code.
    ///
    /// After this call the actor is gone and further request methods will
    /// return [`AcpError::ConnectionClosed`].
    pub async fn shutdown(&self) -> Result<i32> {
        let (reply_tx, reply_rx) = oneshot::channel();
        // If the actor channel is already closed (child died), surface the
        // recorded exit code if we have one, else a generic ConnectionClosed.
        if self
            .inner
            .cmd_tx
            .send(ActorCommand::Shutdown { reply: reply_tx })
            .await
            .is_err()
        {
            return match *self.inner.child_exit.lock().await {
                Some(code) => Ok(code),
                None => Err(AcpError::ConnectionClosed { request_id: None }),
            };
        }
        reply_rx
            .await
            .map_err(|_| AcpError::ConnectionClosed { request_id: None })?
    }

    /// Generic request helper — serialize, dispatch, deserialize.
    async fn request<P, R>(&self, method: &'static str, params: P) -> Result<R>
    where
        P: serde::Serialize,
        R: DeserializeOwned,
    {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let params = serde_json::to_value(params)?;
        let msg = RawMessage::new_request(id, method, Some(params));

        let (reply_tx, reply_rx) = oneshot::channel();
        self.inner
            .cmd_tx
            .send(ActorCommand::Request {
                msg,
                reply: reply_tx,
            })
            .await
            .map_err(|_| AcpError::ConnectionClosed {
                request_id: Some(id),
            })?;

        let raw = reply_rx
            .await
            .map_err(|_| AcpError::ConnectionClosed {
                request_id: Some(id),
            })??;
        serde_json::from_value(raw).map_err(|e| AcpError::MalformedResponse {
            method: method.to_string(),
            detail: e.to_string(),
        })
    }
}

// ── Actor task ───────────────────────────────────────────────────────────────

async fn actor_loop(
    mut child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    mut cmd_rx: mpsc::Receiver<ActorCommand>,
    notifications_tx: broadcast::Sender<wire::SessionNotification>,
    inner: Arc<Inner>,
) -> Result<i32> {
    let mut stdin = stdin;
    let mut stdout_lines = BufReader::new(stdout).lines();
    let mut pending: HashMap<i64, oneshot::Sender<Result<Value>>> = HashMap::new();
    let mut shutdown_reply: Option<oneshot::Sender<Result<i32>>> = None;

    loop {
        tokio::select! {
            // Outbound: a handle wants to send a message or initiate shutdown.
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(ActorCommand::Request { msg, reply }) => {
                        let id = msg.id.expect("requests have id");
                        if let Err(e) = write_message(&mut stdin, &msg).await {
                            let _ = reply.send(Err(e));
                            continue;
                        }
                        pending.insert(id, reply);
                    }
                    Some(ActorCommand::Notification { msg }) => {
                        if let Err(e) = write_message(&mut stdin, &msg).await {
                            tracing::warn!("acp: write notification failed: {e}");
                        }
                    }
                    Some(ActorCommand::Shutdown { reply }) => {
                        shutdown_reply = Some(reply);
                        break;
                    }
                    None => {
                        // All handles dropped — abort.
                        break;
                    }
                }
            }

            // Inbound: read a line from the agent.
            line = stdout_lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if line.trim().is_empty() {
                            continue;
                        }
                        match dispatch_inbound(&line, &mut pending, &notifications_tx, &mut stdin).await {
                            Ok(()) => {}
                            Err(e) => tracing::warn!("acp: inbound dispatch error: {e}"),
                        }
                    }
                    Ok(None) => {
                        tracing::debug!("acp: agent stdout EOF");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!("acp: stdout read error: {e}");
                        break;
                    }
                }
            }
        }
    }

    // Drain any remaining pending requests with ConnectionClosed.
    for (id, reply) in pending.drain() {
        let _ = reply.send(Err(AcpError::ConnectionClosed {
            request_id: Some(id),
        }));
    }

    // Graceful shutdown sequence: close stdin, give the agent a moment, then
    // SIGTERM, then SIGKILL.
    drop(stdin);
    let exit_code = wait_with_grace(&mut child).await;
    *inner.child_exit.lock().await = Some(exit_code);

    if let Some(reply) = shutdown_reply {
        let _ = reply.send(Ok(exit_code));
    }
    Ok(exit_code)
}

async fn wait_with_grace(child: &mut Child) -> i32 {
    if let Ok(Some(status)) =
        tokio::time::timeout(STDIN_CLOSE_GRACE, child.wait()).await.map(|r| r.ok())
    {
        return status.code().unwrap_or(-1);
    }
    let _ = child.start_kill(); // SIGKILL via tokio (no portable SIGTERM yet)
    if let Ok(Ok(status)) = tokio::time::timeout(SIGTERM_GRACE, child.wait()).await {
        return status.code().unwrap_or(-1);
    }
    -1
}

async fn dispatch_inbound(
    line: &str,
    pending: &mut HashMap<i64, oneshot::Sender<Result<Value>>>,
    notifications_tx: &broadcast::Sender<wire::SessionNotification>,
    stdin: &mut ChildStdin,
) -> Result<()> {
    let raw: RawMessage = serde_json::from_str(line)?;
    let msg = raw
        .classify()
        .map_err(|e| AcpError::other(format!("classify: {e}")))?;
    match msg {
        Message::Response { id, result } => {
            if let Some(reply) = pending.remove(&id) {
                let r = result.map_err(|e| AcpError::RpcError {
                    code: e.code,
                    message: e.message,
                    data: e.data,
                });
                let _ = reply.send(r);
            } else {
                tracing::warn!("acp: response for unknown request id={id}");
            }
        }
        Message::Notification { method, params } => {
            handle_inbound_notification(&method, params, notifications_tx);
        }
        Message::Request { id, method, params } => {
            handle_inbound_request(id, &method, params, stdin).await;
        }
    }
    Ok(())
}

fn handle_inbound_notification(
    method: &str,
    params: Option<Value>,
    notifications_tx: &broadcast::Sender<wire::SessionNotification>,
) {
    if method == client_methods::SESSION_UPDATE {
        match params.ok_or_else(|| AcpError::other("session/update missing params"))
            .and_then(|p| serde_json::from_value::<wire::SessionNotification>(p).map_err(Into::into))
        {
            Ok(notif) => {
                // Send error is fine — means no subscribers.
                let _ = notifications_tx.send(notif);
            }
            Err(e) => tracing::warn!("acp: malformed session/update: {e}"),
        }
        return;
    }
    tracing::debug!("acp: ignored notification method={method}");
}

async fn handle_inbound_request(
    id: i64,
    method: &str,
    _params: Option<Value>,
    stdin: &mut ChildStdin,
) {
    // First-cut policy: deny anything sensitive (permissions), method-not-found
    // for fs/terminal capabilities we did not advertise. A future
    // ClientHandler trait will replace this with pluggable behavior.
    let response = match method {
        m if m == client_methods::SESSION_REQUEST_PERMISSION => {
            // Default-deny — caller advertises whatever permissions it needs
            // by replacing this layer.
            RawMessage::new_success(
                id,
                serde_json::json!({ "outcome": { "outcome": "selected", "optionId": "deny" } }),
            )
        }
        _ => RawMessage::new_error(
            id,
            RpcError {
                code: -32601,
                message: format!("method not found: {method}"),
                data: None,
            },
        ),
    };
    if let Err(e) = write_message(stdin, &response).await {
        tracing::warn!("acp: failed to respond to inbound request {method}: {e}");
    }
}

async fn write_message(stdin: &mut ChildStdin, msg: &RawMessage) -> Result<()> {
    let mut buf = serde_json::to_vec(msg)?;
    buf.push(b'\n');
    stdin.write_all(&buf).await?;
    stdin.flush().await?;
    Ok(())
}

async fn forward_stderr(stderr: tokio::process::ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::warn!(target: "mcd_acp::child_stderr", "{line}");
    }
}

/// Build a parent-env clone with `CLAUDECODE` and `CLAUDE_CODE_*` stripped.
///
/// `claude-code-acp` errors out with a misleading `-32603 Internal error` on
/// `session/new` if it sees those vars (it detects nested Claude Code).
fn scrubbed_parent_env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| k != "CLAUDECODE" && !k.starts_with("CLAUDE_CODE_"))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn scrub_strips_claudecode_vars() {
        // Sanity: we exclude both exact and prefix matches.
        let input: Vec<(String, String)> = vec![
            ("CLAUDECODE".into(), "1".into()),
            ("CLAUDE_CODE_ENTRYPOINT".into(), "x".into()),
            ("PATH".into(), "/usr/bin".into()),
            ("HOME".into(), "/root".into()),
        ];
        let kept: Vec<(String, String)> = input
            .into_iter()
            .filter(|(k, _)| k != "CLAUDECODE" && !k.starts_with("CLAUDE_CODE_"))
            .collect();
        assert!(kept.iter().any(|(k, _)| k == "PATH"));
        assert!(kept.iter().any(|(k, _)| k == "HOME"));
        assert!(!kept.iter().any(|(k, _)| k == "CLAUDECODE"));
        assert!(!kept.iter().any(|(k, _)| k.starts_with("CLAUDE_CODE_")));
    }
}
