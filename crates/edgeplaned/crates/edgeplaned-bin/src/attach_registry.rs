//! Process-wide registry of live persistent-session endpoints, keyed by
//! `agent_id`. Populated by the appropriate supervisor when it owns a
//! session; consumed by the attach surfaces (Unix socket gateway, network
//! WS server) so multiple viewers can attach to the same persistent agent
//! without spawning new processes.
//!
//! Two endpoint shapes — selected at registration time by the supervisor:
//!
//! - [`PtyAttachEndpoints`] — byte-stream PTY (claude-code, codex, gemini,
//!   goose). Fan-out via stdout broadcast; unicast stdin/resize; in-band
//!   AgentSignal delivery for peer-message relay.
//! - [`AcpAttachEndpoints`] — JSON-RPC over stdio (claude-agent-acp).
//!   Fan-out via session/update broadcast; signal channel still carries
//!   AgentSignal so the existing peer-message relay routes work unchanged
//!   (the supervisor maps UserInput → session/prompt, Cancel → session/cancel).

use std::collections::HashMap;
use std::sync::Arc;

use edgeplaned_acp::wire::SessionNotification;
use edgeplaned_core::types::AgentSignal;
use tokio::sync::{Mutex, broadcast, mpsc};

use crate::replay_broadcast::ReplayBroadcast;

/// PTY-shaped attach endpoints. Used by the byte-stream supervisor.
#[allow(dead_code)] // some fields not yet consumed by every viewer surface
#[derive(Clone)]
pub struct PtyAttachEndpoints {
    pub stdin_tx: mpsc::Sender<Vec<u8>>,
    pub stdout_broadcast: broadcast::Sender<Vec<u8>>,
    pub resize_tx: mpsc::Sender<(u16, u16)>,
    pub signal_tx: mpsc::Sender<AgentSignal>,
}

/// ACP-shaped attach endpoints. Used by the ACP persistent-session
/// supervisor.
#[derive(Clone)]
pub struct AcpAttachEndpoints {
    /// In-band path for [`AgentSignal`] delivery. The supervisor consumes
    /// this and renders signals into ACP calls:
    /// - `UserInput` / `PeerMessage` → `session/prompt`
    /// - `Cancel` → `session/cancel`
    pub signal_tx: mpsc::Sender<AgentSignal>,
    /// Streaming `session/update` notifications from the agent. Fronted by
    /// a bounded replay buffer (see `replay_broadcast`) so a viewer
    /// attaching mid-session immediately sees the recent conversation
    /// instead of an empty pane. New viewers call
    /// `subscribe_with_replay()`; the snapshot drains first, then live
    /// updates stream — with no overlap.
    pub updates_broadcast: ReplayBroadcast<SessionNotification>,
}

/// Either shape of registered endpoints.
#[derive(Clone)]
pub enum AttachEndpoints {
    Pty(PtyAttachEndpoints),
    Acp(AcpAttachEndpoints),
}

impl AttachEndpoints {
    pub fn signal_tx(&self) -> &mpsc::Sender<AgentSignal> {
        match self {
            AttachEndpoints::Pty(e) => &e.signal_tx,
            AttachEndpoints::Acp(e) => &e.signal_tx,
        }
    }
}

#[derive(Default)]
pub struct AttachRegistry {
    inner: Mutex<HashMap<String, AttachEndpoints>>,
}

impl AttachRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register the endpoints for `agent_id`. Replaces any prior entry.
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
