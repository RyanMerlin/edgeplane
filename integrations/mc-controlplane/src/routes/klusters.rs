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
    models::kluster::{Kluster, KlusterCreate, KlusterUpdate},
    state::AppState,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/missions/{mission_id}/k", get(list_klusters).post(create_kluster))
        .route(
            "/missions/{mission_id}/k/{kluster_id}",
            get(get_kluster).patch(update_kluster).delete(delete_kluster),
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

fn mission_readable(mission_visibility: &str, principal: &Principal, mission_owners: &str, mission_contributors: &str) -> bool {
    if principal.is_admin { return true; }
    if mission_visibility.to_lowercase() == "public" { return true; }
    let id = principal.subject.to_lowercase();
    split_csv(mission_owners).contains(&id) || split_csv(mission_contributors).contains(&id)
}

fn mission_writable(principal: &Principal, mission_owners: &str, mission_contributors: &str) -> bool {
    if principal.is_admin { return true; }
    let id = principal.subject.to_lowercase();
    split_csv(mission_owners).contains(&id) || split_csv(mission_contributors).contains(&id)
}

fn mission_ownable(principal: &Principal, mission_owners: &str) -> bool {
    if principal.is_admin { return true; }
    split_csv(mission_owners).contains(&principal.subject.to_lowercase())
}

#[derive(Deserialize)]
struct ListQuery { limit: Option<i64> }

async fn list_klusters(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);
    let m = sqlx::query("SELECT * FROM mission WHERE id=$1")
        .bind(&mission_id).fetch_optional(&state.db).await;
    let (vis, owners, contribs) = match m {
        Ok(Some(r)) => (
            r.try_get::<String, _>("visibility").unwrap_or_default(),
            r.try_get::<String, _>("owners").unwrap_or_default(),
            r.try_get::<String, _>("contributors").unwrap_or_default(),
        ),
        Ok(None) => return not_found("Mission not found"),
        Err(e) => { tracing::error!("list_klusters fetch mission: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };
    if !mission_readable(&vis, &principal, &owners, &contribs) { return StatusCode::FORBIDDEN.into_response(); }

    match sqlx::query_as::<_, Kluster>(
        "SELECT * FROM kluster WHERE mission_id=$1 ORDER BY updated_at DESC LIMIT $2"
    )
    .bind(&mission_id).bind(limit).fetch_all(&state.db).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => { tracing::error!("list_klusters: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn create_kluster(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
    Json(payload): Json<KlusterCreate>,
) -> impl IntoResponse {
    let m = sqlx::query("SELECT * FROM mission WHERE id=$1")
        .bind(&mission_id).fetch_optional(&state.db).await;
    let (owners, contribs) = match m {
        Ok(Some(r)) => (
            r.try_get::<String, _>("owners").unwrap_or_default(),
            r.try_get::<String, _>("contributors").unwrap_or_default(),
        ),
        Ok(None) => return not_found("Mission not found"),
        Err(e) => { tracing::error!("create_kluster fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };
    if !mission_writable(&principal, &owners, &contribs) { return StatusCode::FORBIDDEN.into_response(); }
    if split_csv(&payload.owners).is_empty() {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": "owners must include at least one owner"}))).into_response();
    }

    let mut id = new_hash_id();
    for _ in 0..5 {
        let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM kluster WHERE id=$1")
            .bind(&id).fetch_optional(&state.db).await.unwrap_or(None);
        if exists.is_none() { break; }
        id = new_hash_id();
    }

    let now = Utc::now().naive_utc();
    match sqlx::query_as::<_, Kluster>(
        r#"INSERT INTO kluster
            (id, mission_id, name, description, owners, contributors, tags, status,
             workstream_md, workstream_version, workstream_created_by, workstream_modified_by,
             workstream_created_at, workstream_modified_at, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'',1,'','',NULL,NULL,$9,$9) RETURNING *"#
    )
    .bind(&id).bind(&mission_id).bind(&payload.name).bind(&payload.description)
    .bind(&payload.owners).bind(&payload.contributors).bind(&payload.tags).bind(&payload.status)
    .bind(now)
    .fetch_one(&state.db).await {
        Ok(k) => (StatusCode::OK, Json(k)).into_response(),
        Err(e) => { tracing::error!("create_kluster insert: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn get_kluster(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((mission_id, kluster_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let m = sqlx::query("SELECT * FROM mission WHERE id=$1")
        .bind(&mission_id).fetch_optional(&state.db).await;
    let (vis, owners, contribs) = match m {
        Ok(Some(r)) => (
            r.try_get::<String, _>("visibility").unwrap_or_default(),
            r.try_get::<String, _>("owners").unwrap_or_default(),
            r.try_get::<String, _>("contributors").unwrap_or_default(),
        ),
        Ok(None) => return not_found("Mission not found"),
        Err(e) => { tracing::error!("get_kluster mission: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };
    if !mission_readable(&vis, &principal, &owners, &contribs) { return StatusCode::FORBIDDEN.into_response(); }

    match sqlx::query_as::<_, Kluster>("SELECT * FROM kluster WHERE id=$1 AND mission_id=$2")
        .bind(&kluster_id).bind(&mission_id).fetch_optional(&state.db).await {
        Ok(Some(k)) => Json(k).into_response(),
        Ok(None) => not_found("Kluster not found"),
        Err(e) => { tracing::error!("get_kluster: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn update_kluster(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((mission_id, kluster_id)): Path<(String, String)>,
    Json(payload): Json<KlusterUpdate>,
) -> impl IntoResponse {
    let m = sqlx::query("SELECT * FROM mission WHERE id=$1")
        .bind(&mission_id).fetch_optional(&state.db).await;
    let (owners, contribs) = match m {
        Ok(Some(r)) => (
            r.try_get::<String, _>("owners").unwrap_or_default(),
            r.try_get::<String, _>("contributors").unwrap_or_default(),
        ),
        Ok(None) => return not_found("Mission not found"),
        Err(e) => { tracing::error!("update_kluster mission: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };
    if !mission_writable(&principal, &owners, &contribs) { return StatusCode::FORBIDDEN.into_response(); }

    let k = sqlx::query_as::<_, Kluster>("SELECT * FROM kluster WHERE id=$1 AND mission_id=$2")
        .bind(&kluster_id).bind(&mission_id).fetch_optional(&state.db).await;
    let kluster = match k {
        Ok(Some(k)) => k,
        Ok(None) => return not_found("Kluster not found"),
        Err(e) => { tracing::error!("update_kluster fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    let name         = payload.name.unwrap_or(kluster.name);
    let description  = payload.description.unwrap_or(kluster.description);
    let new_owners   = payload.owners.unwrap_or(kluster.owners);
    let contributors = payload.contributors.unwrap_or(kluster.contributors);
    let tags         = payload.tags.unwrap_or(kluster.tags);
    let status       = payload.status.unwrap_or(kluster.status);

    if split_csv(&new_owners).is_empty() {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": "owners must include at least one owner"}))).into_response();
    }

    let now = Utc::now().naive_utc();
    match sqlx::query_as::<_, Kluster>(
        "UPDATE kluster SET name=$2, description=$3, owners=$4, contributors=$5, tags=$6, \
         status=$7, updated_at=$8 WHERE id=$1 RETURNING *"
    )
    .bind(&kluster_id).bind(&name).bind(&description).bind(&new_owners)
    .bind(&contributors).bind(&tags).bind(&status).bind(now)
    .fetch_one(&state.db).await {
        Ok(k) => Json(k).into_response(),
        Err(e) => { tracing::error!("update_kluster: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn delete_kluster(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((mission_id, kluster_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let m = sqlx::query("SELECT * FROM mission WHERE id=$1")
        .bind(&mission_id).fetch_optional(&state.db).await;
    let mission_owners = match m {
        Ok(Some(r)) => r.try_get::<String, _>("owners").unwrap_or_default(),
        Ok(None) => return not_found("Mission not found"),
        Err(e) => { tracing::error!("delete_kluster mission: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };
    if !mission_ownable(&principal, &mission_owners) { return StatusCode::FORBIDDEN.into_response(); }

    let k: Option<i32> = sqlx::query_scalar("SELECT 1 FROM kluster WHERE id=$1 AND mission_id=$2")
        .bind(&kluster_id).bind(&mission_id).fetch_optional(&state.db).await.unwrap_or(None);
    if k.is_none() { return not_found("Kluster not found"); }

    // Block if child entities exist
    let task_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task WHERE kluster_id=$1")
        .bind(&kluster_id).fetch_one(&state.db).await.unwrap_or(0);
    if task_count > 0 {
        return (StatusCode::CONFLICT, Json(serde_json::json!({"detail": format!("Kluster has linked entities: {{tasks: {}}}", task_count)}))).into_response();
    }

    let _ = sqlx::query("DELETE FROM kluster WHERE id=$1").bind(&kluster_id).execute(&state.db).await;
    Json(serde_json::json!({"ok": true, "deleted_id": kluster_id})).into_response()
}
