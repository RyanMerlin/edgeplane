//! Wire compatibility test against the real `@zed-industries/claude-code-acp`
//! agent. This is the canary for upstream protocol drift: it exercises the
//! full request/response/notification cycle against the same binary that
//! would run in production.
//!
//! ## Skipping
//!
//! Skips (with a `tracing::warn!`) if either:
//! - the env var `EP_MESH_ACP_SKIP_WIRE` is set, OR
//! - `node` is not on PATH, OR
//! - `claude-code-acp/dist/index.js` cannot be located via `EP_MESH_ACP_JS`
//!   env var or the standard search paths below.
//!
//! ## Search paths for `dist/index.js` (in order)
//!
//! 1. `$EP_MESH_ACP_JS` (full path to `dist/index.js`)
//! 2. `/tmp/acp-smoke/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js`
//!    (the renamed package; current canonical name)
//! 3. `/tmp/acp-smoke/node_modules/@zed-industries/claude-code-acp/dist/index.js`
//!    (legacy name, kept as a fallback while old installs exist)
//!
//! ## Auth pre-req
//!
//! claude-code-acp inherits the host's Claude Code login. Run
//! `claude /login` once on the test machine before running this test.

use std::path::PathBuf;
use std::time::Duration;

use edgeplaned_acp::{
    Agent, ContentBlock, SessionUpdate, SpawnOpts, consts::PROTOCOL_VERSION, schema,
};
use tokio::sync::broadcast::error::RecvError;

/// Returns the CWD to use for the ACP test session.
///
/// Resolved in priority order:
/// 1. `$EP_MESH_ACP_CWD` — set this to any directory you want the test agent
///    to start in (e.g. your project checkout).
/// 2. `$TMPDIR` / `/tmp` — a universally available fallback that works on CI
///    without any pre-created profile directory.
fn profile_cwd() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("EP_MESH_ACP_CWD") {
        return std::path::PathBuf::from(p);
    }
    std::env::var("TMPDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
}

fn skip_reason() -> Option<String> {
    if std::env::var("EP_MESH_ACP_SKIP_WIRE").is_ok() {
        return Some("EP_MESH_ACP_SKIP_WIRE set".into());
    }
    if which::which("node").is_err() {
        return Some("node not on PATH".into());
    }
    if locate_acp_js().is_none() {
        return Some(
            "claude-code-acp dist/index.js not found — set EP_MESH_ACP_JS or `npm i -g @zed-industries/claude-code-acp`".into(),
        );
    }
    let cwd = profile_cwd();
    if !cwd.exists() {
        return Some(format!(
            "test CWD '{}' not present — set EP_MESH_ACP_CWD",
            cwd.display()
        ));
    }
    None
}

fn locate_acp_js() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("EP_MESH_ACP_JS") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    for candidate in [
        "/tmp/acp-smoke/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js",
        "/tmp/acp-smoke/node_modules/@zed-industries/claude-code-acp/dist/index.js",
    ] {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn node_path() -> PathBuf {
    which::which("node").expect("node already verified present")
}

#[tokio::test]
async fn wire_compat_initialize_session_prompt_roundtrip() {
    let _ = tracing_subscriber::fmt::try_init();

    if let Some(reason) = skip_reason() {
        eprintln!("SKIP wire_compat: {reason}");
        return;
    }

    let acp_js = locate_acp_js().expect("locate_acp_js after skip check");
    let opts = SpawnOpts::claude_code_acp(node_path(), acp_js);

    // Spawn — the call should return quickly (process up, pipes wired).
    let agent = tokio::time::timeout(Duration::from_secs(10), Agent::spawn(opts))
        .await
        .expect("spawn timeout")
        .expect("spawn ok");

    // Subscribe BEFORE prompting so we don't miss early `available_commands_update`.
    let mut updates = agent.subscribe_session_updates();

    // 1. initialize
    let init = tokio::time::timeout(
        Duration::from_secs(15),
        agent.initialize(schema::InitializeRequest {
            meta: None,
            protocol_version: schema::ProtocolVersion(PROTOCOL_VERSION as u16),
            client_info: Some(schema::Implementation {
                meta: None,
                name: "edgeplaned-acp wire_compat test".into(),
                title: None,
                version: env!("CARGO_PKG_VERSION").into(),
            }),
            // Defaults: all fs/terminal/auth/nes/elicitation features off.
            // Use Default to keep this test resilient when upstream adds new
            // capability fields (the sync-acp loop's whole point).
            client_capabilities: schema::ClientCapabilities::default(),
        }),
    )
    .await
    .expect("initialize timeout")
    .expect("initialize ok");
    assert_eq!(init.protocol_version.0, PROTOCOL_VERSION as u16);

    // 2. session/new
    let new_session = tokio::time::timeout(
        Duration::from_secs(30),
        agent.new_session(schema::NewSessionRequest {
            meta: None,
            cwd: profile_cwd().to_string_lossy().into_owned(),
            mcp_servers: vec![],
        }),
    )
    .await
    .expect("session/new timeout")
    .expect("session/new ok");
    let session_id = new_session.session_id.clone();

    // 3. session/prompt — collect chunks concurrently, exit collector as soon
    //    as the RPC future resolves (no fixed timeout-based wait).
    let prompt_fut = agent.prompt(schema::PromptRequest {
        meta: None,
        session_id: session_id.clone(),
        prompt: vec![ContentBlock::text("Reply with just the single word PONG.")],
    });
    tokio::pin!(prompt_fut);

    let mut chunks: Vec<String> = vec![];
    let prompt_result = loop {
        tokio::select! {
            biased;
            res = &mut prompt_fut => {
                break res.expect("prompt rpc ok");
            }
            recv = updates.recv() => {
                match recv {
                    Ok(notif) => {
                        if let SessionUpdate::AgentMessageChunk { content } = &notif.update
                            && let Some(text) = content.as_text()
                        {
                            chunks.push(text.to_string());
                        }
                    }
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => continue, // wait for prompt_fut
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(45)) => {
                panic!("prompt did not complete within 45s");
            }
        }
    };

    // 4. Assertions
    assert!(
        matches!(prompt_result.stop_reason, schema::StopReason::EndTurn),
        "expected EndTurn, got {:?}",
        prompt_result.stop_reason
    );
    let combined = chunks.join("").to_uppercase();
    assert!(
        combined.contains("PONG"),
        "expected agent reply to contain PONG, got chunks={chunks:?}"
    );

    // 5. shutdown — verify clean exit code path
    let exit_code = tokio::time::timeout(Duration::from_secs(15), agent.shutdown())
        .await
        .expect("shutdown timeout")
        .expect("shutdown ok");
    eprintln!("agent exited code={exit_code}");
}
