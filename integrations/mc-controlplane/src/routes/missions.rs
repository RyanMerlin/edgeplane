use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use sqlx::postgres::PgRow;
use sqlx::Row;
use std::sync::Arc;

use crate::{
    auth::Principal,
    models::mission::{Mission, MissionCreate, MissionRoleMembership, MissionRoleUpsert, MissionUpdate},
    state::AppState,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/missions", get(list_missions).post(create_mission))
        .route("/missions/{mission_id}", get(get_mission).patch(update_mission).delete(delete_mission))
        .route("/missions/{mission_id}/owner", post(transfer_owner))
        .route("/missions/{mission_id}/roles", get(list_roles).post(upsert_role))
        .route("/missions/{mission_id}/roles/{subject}", delete(delete_role))
}

fn new_hash_id() -> String {
    let bytes: [u8; 6] = rand::random();
    hex::encode(bytes)
}

fn split_csv(s: &str) -> Vec<String> {
    s.split(',').map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect()
}

fn can_read(mission: &Mission, p: &Principal) -> bool {
    if p.is_admin { return true; }
    if mission.visibility.to_lowercase() == "public" { return true; }
    let id = p.subject.to_lowercase();
    split_csv(&mission.owners).contains(&id) || split_csv(&mission.contributors).contains(&id)
}

fn can_write(mission: &Mission, p: &Principal) -> bool {
    if p.is_admin { return true; }
    let id = p.subject.to_lowercase();
    split_csv(&mission.owners).contains(&id) || split_csv(&mission.contributors).contains(&id)
}

fn can_own(mission: &Mission, p: &Principal) -> bool {
    if p.is_admin { return true; }
    let id = p.subject.to_lowercase();
    split_csv(&mission.owners).contains(&id)
}

fn row_to_mission(row: &PgRow) -> Mission {
    Mission {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        owners: row.get("owners"),
        contributors: row.get("contributors"),
        tags: row.get("tags"),
        visibility: row.get("visibility"),
        status: row.get("status"),
        northstar_md: row.get("northstar_md"),
        northstar_version: row.get("northstar_version"),
        northstar_created_by: row.get("northstar_created_by"),
        northstar_modified_by: row.get("northstar_modified_by"),
        northstar_created_at: row.get("northstar_created_at"),
        northstar_modified_at: row.get("northstar_modified_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn row_to_role(row: &PgRow) -> MissionRoleMembership {
    MissionRoleMembership {
        id: row.get("id"),
        mission_id: row.get("mission_id"),
        subject: row.get("subject"),
        role: row.get("role"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}
fn unprocessable(msg: &str) -> axum::response::Response {
    (StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": msg}))).into_response()
}

// ── Handlers ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<i64>,
}

async fn list_missions(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);
    let rows = sqlx::query("SELECT * FROM mission ORDER BY updated_at DESC LIMIT $1")
        .bind(limit)
        .fetch_all(&state.db)
        .await;

    match rows {
        Err(e) => { tracing::error!("list_missions: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
        Ok(rows) => {
            let missions: Vec<Mission> = rows.iter().map(row_to_mission).collect();
            let visible: Vec<&Mission> = missions.iter().filter(|m| can_read(m, &principal)).collect();
            Json(visible).into_response()
        }
    }
}

async fn create_mission(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<MissionCreate>,
) -> impl IntoResponse {
    if payload.name.trim().is_empty() {
        return unprocessable("name is required");
    }
    let owners = if payload.owners.trim().is_empty() && principal.subject != "anonymous" {
        principal.subject.clone()
    } else {
        payload.owners.clone()
    };
    if split_csv(&owners).is_empty() {
        return unprocessable("owners must include at least one owner");
    }

    let mut id = new_hash_id();
    for _ in 0..5 {
        let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM mission WHERE id = $1")
            .bind(&id)
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None);
        if exists.is_none() { break; }
        id = new_hash_id();
    }

    let now = Utc::now().naive_utc();
    let result = sqlx::query(
        r#"INSERT INTO mission
            (id, name, description, owners, contributors, tags, visibility, status,
             northstar_md, northstar_version, northstar_created_by, northstar_modified_by,
             northstar_created_at, northstar_modified_at, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'',1,'','',NULL,NULL,$9,$10)
           RETURNING *"#,
    )
    .bind(&id).bind(payload.name.trim()).bind(&payload.description)
    .bind(&owners).bind(&payload.contributors).bind(&payload.tags)
    .bind(&payload.visibility).bind(&payload.status)
    .bind(now).bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => (StatusCode::OK, Json(row_to_mission(&row))).into_response(),
        Err(e) if e.to_string().contains("unique") || e.to_string().contains("duplicate") => {
            (StatusCode::CONFLICT, Json(serde_json::json!({"detail": "Mission name already exists"}))).into_response()
        }
        Err(e) => { tracing::error!("create_mission: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn get_mission(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT * FROM mission WHERE id = $1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await;

    match row {
        Ok(Some(r)) => {
            let m = row_to_mission(&r);
            if can_read(&m, &principal) { Json(m).into_response() } else { StatusCode::FORBIDDEN.into_response() }
        }
        Ok(None) => not_found("Mission not found"),
        Err(e) => { tracing::error!("get_mission: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn update_mission(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
    Json(payload): Json<MissionUpdate>,
) -> impl IntoResponse {
    let existing = sqlx::query("SELECT * FROM mission WHERE id = $1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await;

    let mission = match existing {
        Ok(Some(r)) => row_to_mission(&r),
        Ok(None) => return not_found("Mission not found"),
        Err(e) => { tracing::error!("update_mission fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    if !can_write(&mission, &principal) { return StatusCode::FORBIDDEN.into_response(); }

    let owners = payload.owners.unwrap_or(mission.owners);
    if split_csv(&owners).is_empty() { return unprocessable("owners must include at least one owner"); }

    let description  = payload.description.unwrap_or(mission.description);
    let contributors = payload.contributors.unwrap_or(mission.contributors);
    let tags         = payload.tags.unwrap_or(mission.tags);
    let visibility   = payload.visibility.unwrap_or(mission.visibility);
    let status       = payload.status.unwrap_or(mission.status);
    let now = Utc::now().naive_utc();

    let result = sqlx::query(
        "UPDATE mission SET description=$2, owners=$3, contributors=$4, tags=$5, \
         visibility=$6, status=$7, updated_at=$8 WHERE id=$1 RETURNING *"
    )
    .bind(&mission_id).bind(&description).bind(&owners).bind(&contributors)
    .bind(&tags).bind(&visibility).bind(&status).bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => Json(row_to_mission(&row)).into_response(),
        Err(e) => { tracing::error!("update_mission: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn delete_mission(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
) -> impl IntoResponse {
    let existing = sqlx::query("SELECT * FROM mission WHERE id = $1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await;

    let mission = match existing {
        Ok(Some(r)) => row_to_mission(&r),
        Ok(None) => return not_found("Mission not found"),
        Err(e) => { tracing::error!("delete_mission fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    if !can_own(&mission, &principal) { return StatusCode::FORBIDDEN.into_response(); }

    let linked: Option<i32> = sqlx::query_scalar("SELECT 1 FROM kluster WHERE mission_id = $1 LIMIT 1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if linked.is_some() {
        return (StatusCode::CONFLICT, Json(serde_json::json!({"detail": "Mission has linked klusters; move or delete klusters first"}))).into_response();
    }

    let _ = sqlx::query("DELETE FROM missionrolemembership WHERE mission_id = $1")
        .bind(&mission_id).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM mission WHERE id = $1")
        .bind(&mission_id).execute(&state.db).await;

    Json(serde_json::json!({"ok": true, "deleted_id": mission_id})).into_response()
}

// ── Role endpoints ────────────────────────────────────────────────────────────

async fn list_roles(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT * FROM mission WHERE id = $1")
        .bind(&mission_id).fetch_optional(&state.db).await;
    match row {
        Ok(Some(r)) => { let m = row_to_mission(&r); if !can_own(&m, &principal) { return StatusCode::FORBIDDEN.into_response(); } }
        Ok(None) => return not_found("Mission not found"),
        Err(e) => { tracing::error!("list_roles fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    }

    let rows = sqlx::query(
        "SELECT * FROM missionrolemembership WHERE mission_id = $1 ORDER BY created_at ASC"
    )
    .bind(&mission_id).fetch_all(&state.db).await;

    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_role).collect::<Vec<_>>()).into_response(),
        Err(e) => { tracing::error!("list_roles query: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn upsert_role(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
    Json(payload): Json<MissionRoleUpsert>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT * FROM mission WHERE id = $1")
        .bind(&mission_id).fetch_optional(&state.db).await;
    match row {
        Ok(Some(r)) => { let m = row_to_mission(&r); if !can_own(&m, &principal) { return StatusCode::FORBIDDEN.into_response(); } }
        Ok(None) => return not_found("Mission not found"),
        Err(e) => { tracing::error!("upsert_role fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    }

    let now = Utc::now().naive_utc();
    let result = sqlx::query(
        r#"INSERT INTO missionrolemembership (mission_id, subject, role, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $4)
           ON CONFLICT (mission_id, subject) DO UPDATE
             SET role = EXCLUDED.role, updated_at = EXCLUDED.updated_at
           RETURNING *"#
    )
    .bind(&mission_id).bind(&payload.subject).bind(&payload.role).bind(now)
    .fetch_one(&state.db).await;

    match result {
        Ok(row) => Json(row_to_role(&row)).into_response(),
        Err(e) => { tracing::error!("upsert_role insert: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn transfer_owner(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    if !principal.is_admin {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"detail": "admin required"}))).into_response();
    }
    let new_owner = match payload.get("new_owner").and_then(|v| v.as_str()) {
        Some(o) if !o.is_empty() => o.to_string(),
        _ => return (StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": "new_owner is required"}))).into_response(),
    };
    let now = chrono::Utc::now().naive_utc();
    match sqlx::query(
        "UPDATE mission SET owners=$2, updated_at=$3 WHERE id=$1 RETURNING *"
    )
    .bind(&mission_id).bind(&new_owner).bind(now)
    .fetch_optional(&state.db).await {
        Ok(Some(row)) => Json(row_to_mission(&row)).into_response(),
        Ok(None) => not_found("Mission not found"),
        Err(e) => { tracing::error!("transfer_owner: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn delete_role(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((mission_id, subject)): Path<(String, String)>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT * FROM mission WHERE id = $1")
        .bind(&mission_id).fetch_optional(&state.db).await;
    match row {
        Ok(Some(r)) => { let m = row_to_mission(&r); if !can_own(&m, &principal) { return StatusCode::FORBIDDEN.into_response(); } }
        Ok(None) => return not_found("Mission not found"),
        Err(e) => { tracing::error!("delete_role fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    }

    let result = sqlx::query(
        "DELETE FROM missionrolemembership WHERE mission_id = $1 AND subject = $2"
    )
    .bind(&mission_id).bind(&subject)
    .execute(&state.db).await;

    match result {
        Ok(r) if r.rows_affected() > 0 => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(_) => not_found("Role assignment not found"),
        Err(e) => { tracing::error!("delete_role exec: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}
