use jsonwebtoken::{DecodingKey, EncodingKey};
use sqlx::PgPool;

pub struct AppState {
    pub db: PgPool,
    pub node: NodeInfo,
    /// Optional upstream URL — unknown routes are forwarded here (proxy mode).
    pub api_proxy: Option<String>,
    /// RS256 private key for signing node JWTs.
    pub jwt_encoding_key: EncodingKey,
    /// RS256 public key for verifying node JWTs.
    pub jwt_decoding_key: DecodingKey,
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
