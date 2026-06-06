use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::Utc;
use rand::random;
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;

use crate::{
    auth::Principal,
    models::mission::{Mission, MissionCreate, MissionUpdate},
    state::AppState,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/domains/{domain_id}/m", get(list_missions).post(create_mission))
        .route(
            "/domains/{domain_id}/m/{mission_id}",
            get(get_mission).patch(update_mission).delete(delete_mission),
        )
}

fn new_hash_id() -> String {
    hex::encode(random::<[u8; 6]>())
}

fn split_csv(s: &str) -> Vec<String> {
    s.split(',').map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect()
}

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn domain_readable(domain_visibility: &str, principal: &Principal, domain_owners: &str, domain_contributors: &str) -> bool {
    if principal.is_admin { return true; }
    if domain_visibility.to_lowercase() == "public" { return true; }
    let id = principal.subject.to_lowercase();
    split_csv(domain_owners).contains(&id) || split_csv(domain_contributors).contains(&id)
}

fn domain_writable(principal: &Principal, domain_owners: &str, domain_contributors: &str) -> bool {
    if principal.is_admin { return true; }
    let id = principal.subject.to_lowercase();
    split_csv(domain_owners).contains(&id) || split_csv(domain_contributors).contains(&id)
}

fn domain_ownable(principal: &Principal, domain_owners: &str) -> bool {
    if principal.is_admin { return true; }
    split_csv(domain_owners).contains(&principal.subject.to_lowercase())
}

#[derive(Deserialize)]
struct ListQuery { limit: Option<i64> }

async fn list_missions(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);
    let m = sqlx::query("SELECT * FROM domain WHERE id=$1")
        .bind(&domain_id).fetch_optional(&state.db).await;
    let (vis, owners, contribs) = match m {
        Ok(Some(r)) => (
            r.try_get::<String, _>("visibility").unwrap_or_default(),
            r.try_get::<String, _>("owners").unwrap_or_default(),
            r.try_get::<String, _>("contributors").unwrap_or_default(),
        ),
        Ok(None) => return not_found("Domain not found"),
        Err(e) => { tracing::error!("list_missions fetch domain: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };
    if !domain_readable(&vis, &principal, &owners, &contribs) { return StatusCode::FORBIDDEN.into_response(); }

    match sqlx::query_as::<_, Mission>(
        "SELECT * FROM mission WHERE domain_id=$1 ORDER BY updated_at DESC LIMIT $2"
    )
    .bind(&domain_id).bind(limit).fetch_all(&state.db).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => { tracing::error!("list_missions: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn create_mission(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
    Json(payload): Json<MissionCreate>,
) -> impl IntoResponse {
    let m = sqlx::query("SELECT * FROM domain WHERE id=$1")
        .bind(&domain_id).fetch_optional(&state.db).await;
    let (owners, contribs) = match m {
        Ok(Some(r)) => (
            r.try_get::<String, _>("owners").unwrap_or_default(),
            r.try_get::<String, _>("contributors").unwrap_or_default(),
        ),
        Ok(None) => return not_found("Domain not found"),
        Err(e) => { tracing::error!("create_mission fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };
    if !domain_writable(&principal, &owners, &contribs) { return StatusCode::FORBIDDEN.into_response(); }
    // Mirror the domain handler: an empty owners defaults to the authenticated
    // caller's subject (the Principal extractor guarantees one). Lets a service
    // account (e.g. edgeplaned's intake-mission bootstrap) omit owners without a
    // 422, instead of every caller having to echo its own subject.
    let mission_owners = if payload.owners.trim().is_empty() {
        principal.subject.clone()
    } else {
        payload.owners.clone()
    };
    if split_csv(&mission_owners).is_empty() {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": "owners must include at least one owner"}))).into_response();
    }

    let mut id = new_hash_id();
    for _ in 0..5 {
        let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM mission WHERE id=$1")
            .bind(&id).fetch_optional(&state.db).await.unwrap_or(None);
        if exists.is_none() { break; }
        id = new_hash_id();
    }

    let now = Utc::now().naive_utc();
    // If workstream_md is provided, stamp the created_by/at metadata too so
    // it's not orphaned. If it's empty (the historic default), leave the
    // workstream metadata empty as before.
    let ws_created_by: &str = if payload.workstream_md.is_empty() { "" } else { principal.subject.as_str() };
    let ws_created_at: Option<chrono::NaiveDateTime> = if payload.workstream_md.is_empty() { None } else { Some(now) };
    match sqlx::query_as::<_, Mission>(
        r#"INSERT INTO mission
            (id, domain_id, name, description, owners, contributors, tags, status,
             workstream_md, workstream_version, workstream_created_by, workstream_modified_by,
             workstream_created_at, workstream_modified_at, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,1,$10,$10,$11,$11,$12,$12) RETURNING *"#
    )
    .bind(&id).bind(&domain_id).bind(&payload.name).bind(&payload.description)
    .bind(&mission_owners).bind(&payload.contributors).bind(&payload.tags).bind(&payload.status)
    .bind(&payload.workstream_md)
    .bind(ws_created_by)
    .bind(ws_created_at)
    .bind(now)
    .fetch_one(&state.db).await {
        Ok(k) => (StatusCode::OK, Json(k)).into_response(),
        Err(e) => { tracing::error!("create_mission insert: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn get_mission(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((domain_id, mission_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let m = sqlx::query("SELECT * FROM domain WHERE id=$1")
        .bind(&domain_id).fetch_optional(&state.db).await;
    let (vis, owners, contribs) = match m {
        Ok(Some(r)) => (
            r.try_get::<String, _>("visibility").unwrap_or_default(),
            r.try_get::<String, _>("owners").unwrap_or_default(),
            r.try_get::<String, _>("contributors").unwrap_or_default(),
        ),
        Ok(None) => return not_found("Domain not found"),
        Err(e) => { tracing::error!("get_mission domain: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };
    if !domain_readable(&vis, &principal, &owners, &contribs) { return StatusCode::FORBIDDEN.into_response(); }

    match sqlx::query_as::<_, Mission>("SELECT * FROM mission WHERE id=$1 AND domain_id=$2")
        .bind(&mission_id).bind(&domain_id).fetch_optional(&state.db).await {
        Ok(Some(k)) => Json(k).into_response(),
        Ok(None) => not_found("Mission not found"),
        Err(e) => { tracing::error!("get_mission: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn update_mission(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((domain_id, mission_id)): Path<(String, String)>,
    Json(payload): Json<MissionUpdate>,
) -> impl IntoResponse {
    let m = sqlx::query("SELECT * FROM domain WHERE id=$1")
        .bind(&domain_id).fetch_optional(&state.db).await;
    let (owners, contribs) = match m {
        Ok(Some(r)) => (
            r.try_get::<String, _>("owners").unwrap_or_default(),
            r.try_get::<String, _>("contributors").unwrap_or_default(),
        ),
        Ok(None) => return not_found("Domain not found"),
        Err(e) => { tracing::error!("update_mission domain: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };
    if !domain_writable(&principal, &owners, &contribs) { return StatusCode::FORBIDDEN.into_response(); }

    let k = sqlx::query_as::<_, Mission>("SELECT * FROM mission WHERE id=$1 AND domain_id=$2")
        .bind(&mission_id).bind(&domain_id).fetch_optional(&state.db).await;
    let mission = match k {
        Ok(Some(k)) => k,
        Ok(None) => return not_found("Mission not found"),
        Err(e) => { tracing::error!("update_mission fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    let name         = payload.name.unwrap_or(mission.name);
    let description  = payload.description.unwrap_or(mission.description);
    let new_owners   = payload.owners.unwrap_or(mission.owners);
    let contributors = payload.contributors.unwrap_or(mission.contributors);
    let tags         = payload.tags.unwrap_or(mission.tags);
    let status       = payload.status.unwrap_or(mission.status);

    if split_csv(&new_owners).is_empty() {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": "owners must include at least one owner"}))).into_response();
    }

    let now = Utc::now().naive_utc();
    match sqlx::query_as::<_, Mission>(
        "UPDATE mission SET name=$2, description=$3, owners=$4, contributors=$5, tags=$6, \
         status=$7, updated_at=$8 WHERE id=$1 RETURNING *"
    )
    .bind(&mission_id).bind(&name).bind(&description).bind(&new_owners)
    .bind(&contributors).bind(&tags).bind(&status).bind(now)
    .fetch_one(&state.db).await {
        Ok(k) => Json(k).into_response(),
        Err(e) => { tracing::error!("update_mission: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn delete_mission(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((domain_id, mission_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let m = sqlx::query("SELECT * FROM domain WHERE id=$1")
        .bind(&domain_id).fetch_optional(&state.db).await;
    let domain_owners = match m {
        Ok(Some(r)) => r.try_get::<String, _>("owners").unwrap_or_default(),
        Ok(None) => return not_found("Domain not found"),
        Err(e) => { tracing::error!("delete_mission domain: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };
    if !domain_ownable(&principal, &domain_owners) { return StatusCode::FORBIDDEN.into_response(); }

    let k: Option<i32> = sqlx::query_scalar("SELECT 1 FROM mission WHERE id=$1 AND domain_id=$2")
        .bind(&mission_id).bind(&domain_id).fetch_optional(&state.db).await.unwrap_or(None);
    if k.is_none() { return not_found("Mission not found"); }

    // Block if child entities exist
    let task_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task WHERE mission_id=$1")
        .bind(&mission_id).fetch_one(&state.db).await.unwrap_or(0);
    if task_count > 0 {
        return (StatusCode::CONFLICT, Json(serde_json::json!({"detail": format!("Mission has linked entities: {{tasks: {}}}", task_count)}))).into_response();
    }

    let _ = sqlx::query("DELETE FROM mission WHERE id=$1").bind(&mission_id).execute(&state.db).await;
    Json(serde_json::json!({"ok": true, "deleted_id": mission_id})).into_response()
}
