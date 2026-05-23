use axum::{routing::get, Json, Router};
use std::sync::Arc;

use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/schema-pack", get(get_schema_pack))
}

async fn get_schema_pack() -> Json<serde_json::Value> {
    Json(serde_json::json!({"loaded": false, "schema_pack": {}}))
}
