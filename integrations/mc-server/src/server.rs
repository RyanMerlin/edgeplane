use axum::Router;
use sqlx::PgPool;
use std::sync::Arc;

use crate::{routes, state::{AppState, NodeInfo}};

#[derive(Default, Clone)]
pub struct AppConfig {
    pub node_id: u64,
    pub advertise_url: Option<String>,
}

pub fn build_app(db: PgPool, config: AppConfig) -> Router {
    let state = Arc::new(AppState {
        db,
        node: NodeInfo {
            node_id: config.node_id,
            advertise_url: config.advertise_url.clone(),
            role: "standalone",
            term: 0,
            leader_id: None,
        },
    });

    routes::build_router().with_state(state)
}
