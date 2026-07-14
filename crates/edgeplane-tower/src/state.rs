use jsonwebtoken::{DecodingKey, EncodingKey};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Instant;

pub type NodeScopeCache = Arc<RwLock<HashMap<String, (Instant, Vec<String>)>>>;

pub struct AppState {
    pub db: PgPool,
    pub node: NodeInfo,
    /// Optional upstream URL — unknown routes are forwarded here (proxy mode).
    pub api_proxy: Option<String>,
    /// RS256 private key for signing node JWTs.
    pub jwt_encoding_key: EncodingKey,
    /// RS256 public key for verifying node JWTs.
    pub jwt_decoding_key: DecodingKey,
    /// Lowercased operator emails whose user-session principals resolve to
    /// `is_admin = true`. Populated from `EP_ADMIN_EMAILS` at startup.
    pub admin_emails: HashSet<String>,
    /// IdP group names (exact, case-sensitive) whose members resolve to
    /// `is_admin = true`. Populated from `EP_ADMIN_GROUPS` at startup. This is
    /// the preferred, group-based admin path; `admin_emails` remains a fallback.
    pub admin_groups: HashSet<String>,
    /// Per-node dynamic domain scope cache for node JWTs. Cached entries are
    /// short-lived and invalidated when meshagent assignments change.
    pub node_scope_cache: NodeScopeCache,
}

/// Static node identity — populated from CLI args at startup.
/// When Raft is not running, term=0 and role="standalone".
#[derive(Clone, Debug, serde::Serialize)]
pub struct NodeInfo {
    pub node_id: u64,
    pub advertise_url: Option<String>,
    pub role: &'static str,
    pub term: u64,
    pub leader_id: Option<u64>,
}
