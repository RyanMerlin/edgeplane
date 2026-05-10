/// Process-wide registry of live persistent-session endpoints, keyed by
/// `agent_id`. Populated by `session_supervisor` when it owns a PTY; consumed
/// by `attach_gateway` (Unix socket) and (in Phase 2a) `attach_ws` (network
/// WS server) so multiple viewers can attach to the same persistent agent
/// without spawning new processes.
///
/// `stdout_broadcast` is fan-out — each attach calls `subscribe()`.
/// `stdin_tx`/`resize_tx` are unicast (last-writer-wins for stdin is fine —
/// persistent sessions are user-driven and only one human steers at a time).
/// `signal_tx` is the in-band path for `AgentSignal` delivery from the message
/// relay; the supervisor consumes it and writes the rendered prompt to stdin.
use std::collections::HashMap;
use std::sync::Arc;

use mc_mesh_core::types::AgentSignal;
use tokio::sync::{Mutex, broadcast, mpsc};

// stdin_tx / stdout_broadcast / resize_tx are wired up in Phase 1 by the
// session supervisor and consumed in Phase 2a by the network attach WS server
// (and updated attach_gateway). Suppress the dead-code warning until that lands.
#[allow(dead_code)]
#[derive(Clone)]
pub struct AttachEndpoints {
    pub stdin_tx: mpsc::Sender<Vec<u8>>,
    pub stdout_broadcast: broadcast::Sender<Vec<u8>>,
    pub resize_tx: mpsc::Sender<(u16, u16)>,
    pub signal_tx: mpsc::Sender<AgentSignal>,
}

#[derive(Default)]
pub struct AttachRegistry {
    inner: Mutex<HashMap<String, AttachEndpoints>>,
}

impl AttachRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub async fn register(&self, agent_id: String, endpoints: AttachEndpoints) {
        self.inner.lock().await.insert(agent_id, endpoints);
    }

    pub async fn unregister(&self, agent_id: &str) {
        self.inner.lock().await.remove(agent_id);
    }

    pub async fn get(&self, agent_id: &str) -> Option<AttachEndpoints> {
        self.inner.lock().await.get(agent_id).cloned()
    }
}
