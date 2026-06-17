pub mod agents;
pub mod artifacts;
pub mod auth;
pub mod budgets;
pub mod chat_integrations;
pub mod docs;
pub mod event_triggers;
pub mod explorer;
pub mod feedback;
pub mod google_chat_integrations;
pub mod health;
pub mod hooks;
pub mod ingestion;
pub mod missions;
pub mod mcp;
pub mod domains;
pub mod oidc_web;
pub mod onboarding;
pub mod ops;
pub mod persistence;
pub mod profiles;
pub mod raft;
pub mod remotectl;
pub mod runs;
pub mod runtime;
pub mod scheduled_jobs;
pub mod schema_pack;
pub mod search;
pub mod slack_integrations;
pub mod tasks;
pub mod webhooks_tailscale;
pub mod teams_integrations;
pub mod work;

use axum::Router;
use std::sync::Arc;

use crate::state::AppState;

pub fn build_router() -> Router<Arc<AppState>> {
    Router::new()
        .merge(health::router())
        .merge(raft::router())
        .merge(auth::router())
        .merge(oidc_web::router())
        .merge(domains::router())
        .merge(agents::router())
        .merge(missions::router())
        .merge(tasks::router())
        .merge(runs::router())
        .merge(profiles::router())
        .merge(hooks::router())
        .merge(scheduled_jobs::router())
        .merge(work::router())
        .merge(runtime::router())
        .merge(budgets::router())
        .merge(event_triggers::router())
        .merge(feedback::router())
        .merge(onboarding::router())
        .merge(remotectl::router())
        .merge(artifacts::router())
        .merge(docs::router())
        .merge(persistence::router())
        .merge(schema_pack::router())
        .merge(chat_integrations::router())
        .merge(ingestion::router())
        .merge(search::router())
        .merge(google_chat_integrations::router())
        .merge(teams_integrations::router())
        .merge(explorer::router())
        .merge(mcp::router())
        .merge(ops::router())
        .merge(slack_integrations::router())
        .merge(webhooks_tailscale::router())
}
