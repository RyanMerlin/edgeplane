//! Process-wide registry of live persistent-session endpoints, keyed by
//! `agent_id`. Populated by the appropriate supervisor when it owns a
//! session; consumed by the attach surfaces (Unix socket gateway, network
//! WS server) so multiple viewers can attach to the same persistent agent
//! without spawning new processes.
//!
//! Two endpoint shapes — selected at registration time by the supervisor:
//!
//! - [`PtyAttachEndpoints`] — byte-stream PTY (claude-code, codex, gemini).
//!   Fan-out via stdout broadcast; unicast stdin/resize; in-band
//!   AgentSignal delivery for peer-message relay.
//! - [`AcpAttachEndpoints`] — JSON-RPC over stdio (claude-agent-acp).
//!   Fan-out via session/update broadcast; signal channel still carries
//!   AgentSignal so the existing peer-message relay routes work unchanged
//!   (the supervisor maps UserInput → session/prompt, Cancel → session/cancel).
//!
//! ## Federated alias resolution
//!
//! In federated mode, a controlplane agent has a `public_id` (e.g.
//! `"my-agent-engineer-708650f1"`) that differs from the local fleet-import
//! key under which the PTY bridge was registered (e.g. `"engineer"`).
//! The reconciler calls [`AttachRegistry::register_alias`] to map the
//! public_id → local key so that an attach request for the controlplane
//! id reaches the existing live bridge without re-spawning anything.
//!
//! Aliases are secondary: if `public_id` is also registered directly,
//! the direct entry wins. Aliases are cleared when a direct entry is
//! unregistered under the alias target id (to avoid dangling forwards).

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

/// Inner state, kept behind a single lock for atomicity.
#[derive(Default)]
struct AttachRegistryInner {
    /// Direct registrations: agent_id → endpoints.
    endpoints: HashMap<String, AttachEndpoints>,
    /// Alias map: `public_id` → `local_id`. Set by the reconciler for
    /// federated agents so a web attach for `public_id` resolves to the
    /// bridge registered under `local_id`.
    aliases: HashMap<String, String>,
}

#[derive(Default)]
pub struct AttachRegistry {
    inner: Mutex<AttachRegistryInner>,
}

impl AttachRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register the endpoints for `agent_id`. Replaces any prior entry.
    pub async fn register(&self, agent_id: String, endpoints: AttachEndpoints) {
        self.inner
            .lock()
            .await
            .endpoints
            .insert(agent_id, endpoints);
    }

    /// Remove the endpoint entry for `agent_id` and any aliases that
    /// forward to it (to avoid dangling alias pointers).
    pub async fn unregister(&self, agent_id: &str) {
        let mut guard = self.inner.lock().await;
        guard.endpoints.remove(agent_id);
        // Drop any alias whose target was this agent_id.
        guard
            .aliases
            .retain(|_alias, target| target.as_str() != agent_id);
    }

    /// Register `alias_id` as an alternate lookup key that resolves to
    /// the endpoint already registered under `canonical_id`.
    ///
    /// Used by the reconciler to map a controlplane `public_id` (e.g.
    /// `"my-agent-engineer-708650f1"`) to the bridge registered under the
    /// local fleet-import key (e.g. `"engineer"`). `get(alias_id)` will
    /// then return the same endpoints as `get(canonical_id)` — without
    /// disturbing the live PTY bridge or the running agent.
    ///
    /// If `canonical_id` is later unregistered, the alias is pruned
    /// automatically.
    pub async fn register_alias(&self, alias_id: String, canonical_id: String) {
        let mut guard = self.inner.lock().await;
        guard.aliases.insert(alias_id, canonical_id);
    }

    /// Look up endpoints for `agent_id`. Tries direct registration first;
    /// falls back to the alias map if no direct entry exists.
    pub async fn get(&self, agent_id: &str) -> Option<AttachEndpoints> {
        let guard = self.inner.lock().await;
        if let Some(ep) = guard.endpoints.get(agent_id) {
            return Some(ep.clone());
        }
        // Alias fallback: resolve public_id → local_id → endpoints.
        if let Some(canonical_id) = guard.aliases.get(agent_id) {
            return guard.endpoints.get(canonical_id).cloned();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    fn make_pty_endpoints() -> AttachEndpoints {
        let (stdin_tx, _) = mpsc::channel(1);
        let (stdout_broadcast, _) = broadcast::channel(1);
        let (resize_tx, _) = mpsc::channel(1);
        let (signal_tx, _) = mpsc::channel(1);
        AttachEndpoints::Pty(PtyAttachEndpoints {
            stdin_tx,
            stdout_broadcast,
            resize_tx,
            signal_tx,
        })
    }

    /// An alias for a registered key resolves to its endpoints.
    #[tokio::test]
    async fn alias_resolves_to_registered_endpoints() {
        let registry = AttachRegistry::new();
        registry
            .register("engineer".to_string(), make_pty_endpoints())
            .await;
        registry
            .register_alias(
                "my-agent-engineer-708650f1".to_string(),
                "engineer".to_string(),
            )
            .await;

        // Direct lookup still works.
        assert!(
            registry.get("engineer").await.is_some(),
            "direct lookup under local key must work"
        );
        // Alias lookup resolves to the same bridge.
        assert!(
            registry.get("my-agent-engineer-708650f1").await.is_some(),
            "alias lookup under public_id must resolve"
        );
        // Unknown key returns None.
        assert!(registry.get("unknown-agent").await.is_none());
    }

    /// Unregistering the canonical id prunes the alias.
    #[tokio::test]
    async fn unregister_canonical_prunes_alias() {
        let registry = AttachRegistry::new();
        registry
            .register("engineer".to_string(), make_pty_endpoints())
            .await;
        registry
            .register_alias("my-agent-engineer-hash".to_string(), "engineer".to_string())
            .await;

        registry.unregister("engineer").await;

        assert!(
            registry.get("engineer").await.is_none(),
            "canonical should be gone"
        );
        assert!(
            registry.get("my-agent-engineer-hash").await.is_none(),
            "alias pointing at unregistered canonical should also be gone"
        );
    }

    /// Direct registration takes precedence over any alias pointing to the
    /// same id (the direct entry wins).
    #[tokio::test]
    async fn direct_registration_wins_over_alias() {
        let registry = AttachRegistry::new();
        // Register "a" pointing as alias to "b".
        registry
            .register("b".to_string(), make_pty_endpoints())
            .await;
        registry
            .register_alias("a".to_string(), "b".to_string())
            .await;

        // Now also register "a" directly — it should shadow the alias.
        registry
            .register("a".to_string(), make_pty_endpoints())
            .await;

        // Both are reachable; "a" direct entry is returned (same codepath).
        assert!(registry.get("a").await.is_some());
        assert!(registry.get("b").await.is_some());
    }
}
