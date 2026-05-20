use axum::{extract::State, routing::get, Json, Router};
use std::sync::Arc;

use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/raft/status", get(raft_status))
}

async fn raft_status(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "node_id":      state.node.node_id,
        "advertise_url": state.node.advertise_url,
        "role":         state.node.role,
        "term":         state.node.term,
        "leader_id":    state.node.leader_id,
    }))
}
