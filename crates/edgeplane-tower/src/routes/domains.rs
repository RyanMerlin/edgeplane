use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::Utc;
use serde::Deserialize;
use sqlx::Row;
use sqlx::postgres::PgRow;
use std::sync::Arc;

use crate::{
    auth::{Principal, authorized_for_domain, split_csv},
    models::domain::{Domain, DomainCreate, DomainUpdate, NorthstarUpdate},
    state::AppState,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/domains", get(list_domains).post(create_domain))
        .route(
            "/domains/{domain_id}",
            get(get_domain).patch(update_domain).delete(delete_domain),
        )
        .route(
            "/domains/{domain_id}/northstar",
            get(get_domain_northstar_handler).put(put_domain_northstar_handler),
        )
        .route("/domains/{domain_id}/owner", post(transfer_owner))
}

fn new_hash_id() -> String {
    let bytes: [u8; 6] = rand::random();
    hex::encode(bytes)
}

fn can_read(domain: &Domain, p: &Principal) -> bool {
    if domain.visibility.to_lowercase() == "public" {
        return true;
    }
    authorized_for_domain(domain, p)
}

fn can_write(domain: &Domain, p: &Principal) -> bool {
    authorized_for_domain(domain, p)
}

fn can_own(domain: &Domain, p: &Principal) -> bool {
    if p.is_admin {
        return true;
    }
    split_csv(&domain.owners).contains(&p.subject.to_lowercase())
}

fn row_to_domain(row: &PgRow) -> Domain {
    Domain {
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
        northstar_s3_path: row.try_get("northstar_s3_path").unwrap_or(None),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn not_found(msg: &str) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"detail": msg})),
    )
        .into_response()
}
fn unprocessable(msg: &str) -> axum::response::Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(serde_json::json!({"detail": msg})),
    )
        .into_response()
}

// ── Handlers ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<i64>,
}

async fn list_domains(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);
    let rows = sqlx::query("SELECT * FROM domain ORDER BY updated_at DESC LIMIT $1")
        .bind(limit)
        .fetch_all(&state.db)
        .await;

    match rows {
        Err(e) => {
            tracing::error!("list_domains: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        Ok(rows) => {
            let domains: Vec<Domain> = rows.iter().map(row_to_domain).collect();
            let visible: Vec<&Domain> =
                domains.iter().filter(|m| can_read(m, &principal)).collect();
            Json(visible).into_response()
        }
    }
}

async fn create_domain(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<DomainCreate>,
) -> impl IntoResponse {
    if payload.name.trim().is_empty() {
        return unprocessable("name is required");
    }
    // Principal extractor guarantees an authenticated subject (Phase 1.5);
    // an empty payload.owners now defaults to the caller's subject.
    let owners = if payload.owners.trim().is_empty() {
        principal.subject.clone()
    } else {
        payload.owners.clone()
    };
    if split_csv(&owners).is_empty() {
        return unprocessable("owners must include at least one owner");
    }

    let mut id = new_hash_id();
    for _ in 0..5 {
        let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM domain WHERE id = $1")
            .bind(&id)
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None);
        if exists.is_none() {
            break;
        }
        id = new_hash_id();
    }

    let now = Utc::now().naive_utc();
    let result = sqlx::query(
        r#"INSERT INTO domain
            (id, name, description, owners, contributors, tags, visibility, status,
             northstar_md, northstar_version, northstar_created_by, northstar_modified_by,
             northstar_created_at, northstar_modified_at, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'',1,'','',NULL,NULL,$9,$10)
           RETURNING *"#,
    )
    .bind(&id)
    .bind(payload.name.trim())
    .bind(&payload.description)
    .bind(&owners)
    .bind(&payload.contributors)
    .bind(&payload.tags)
    .bind(&payload.visibility)
    .bind(&payload.status)
    .bind(now)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => (StatusCode::OK, Json(row_to_domain(&row))).into_response(),
        Err(e) if e.to_string().contains("unique") || e.to_string().contains("duplicate") => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"detail": "Domain name already exists"})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("create_domain: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_domain(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT * FROM domain WHERE id = $1")
        .bind(&domain_id)
        .fetch_optional(&state.db)
        .await;

    match row {
        Ok(Some(r)) => {
            let m = row_to_domain(&r);
            if can_read(&m, &principal) {
                Json(m).into_response()
            } else {
                StatusCode::FORBIDDEN.into_response()
            }
        }
        Ok(None) => not_found("Domain not found"),
        Err(e) => {
            tracing::error!("get_domain: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn update_domain(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
    Json(payload): Json<DomainUpdate>,
) -> impl IntoResponse {
    let existing = sqlx::query("SELECT * FROM domain WHERE id = $1")
        .bind(&domain_id)
        .fetch_optional(&state.db)
        .await;

    let domain = match existing {
        Ok(Some(r)) => row_to_domain(&r),
        Ok(None) => return not_found("Domain not found"),
        Err(e) => {
            tracing::error!("update_domain fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if !can_write(&domain, &principal) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let owners = payload.owners.unwrap_or(domain.owners);
    if split_csv(&owners).is_empty() {
        return unprocessable("owners must include at least one owner");
    }

    let description = payload.description.unwrap_or(domain.description);
    let contributors = payload.contributors.unwrap_or(domain.contributors);
    let tags = payload.tags.unwrap_or(domain.tags);
    let visibility = payload.visibility.unwrap_or(domain.visibility);
    let status = payload.status.unwrap_or(domain.status);
    let now = Utc::now().naive_utc();

    let result = sqlx::query(
        "UPDATE domain SET description=$2, owners=$3, contributors=$4, tags=$5, \
         visibility=$6, status=$7, updated_at=$8 WHERE id=$1 RETURNING *",
    )
    .bind(&domain_id)
    .bind(&description)
    .bind(&owners)
    .bind(&contributors)
    .bind(&tags)
    .bind(&visibility)
    .bind(&status)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => Json(row_to_domain(&row)).into_response(),
        Err(e) => {
            tracing::error!("update_domain: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn delete_domain(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
) -> impl IntoResponse {
    let existing = sqlx::query("SELECT * FROM domain WHERE id = $1")
        .bind(&domain_id)
        .fetch_optional(&state.db)
        .await;

    let domain = match existing {
        Ok(Some(r)) => row_to_domain(&r),
        Ok(None) => return not_found("Domain not found"),
        Err(e) => {
            tracing::error!("delete_domain fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if !can_own(&domain, &principal) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let linked: Option<i32> =
        sqlx::query_scalar("SELECT 1 FROM mission WHERE domain_id = $1 LIMIT 1")
            .bind(&domain_id)
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None);
    if linked.is_some() {
        return (StatusCode::CONFLICT, Json(serde_json::json!({"detail": "Domain has linked missions; move or delete missions first"}))).into_response();
    }

    let _ = sqlx::query("DELETE FROM domain WHERE id = $1")
        .bind(&domain_id)
        .execute(&state.db)
        .await;

    Json(serde_json::json!({"ok": true, "deleted_id": domain_id})).into_response()
}

async fn transfer_owner(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    if !principal.is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "admin required"})),
        )
            .into_response();
    }
    let new_owner = match payload.get("new_owner").and_then(|v| v.as_str()) {
        Some(o) if !o.is_empty() => o.to_string(),
        _ => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"detail": "new_owner is required"})),
            )
                .into_response();
        }
    };
    let now = chrono::Utc::now().naive_utc();
    match sqlx::query("UPDATE domain SET owners=$2, updated_at=$3 WHERE id=$1 RETURNING *")
        .bind(&domain_id)
        .bind(&new_owner)
        .bind(now)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(row)) => Json(row_to_domain(&row)).into_response(),
        Ok(None) => not_found("Domain not found"),
        Err(e) => {
            tracing::error!("transfer_owner: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ── Northstar document endpoints ──────────────────────────────────────────────

async fn get_domain_northstar_handler(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT * FROM domain WHERE id = $1")
        .bind(&domain_id)
        .fetch_optional(&state.db)
        .await;

    match row {
        Ok(Some(r)) => {
            let domain = row_to_domain(&r);
            if !can_read(&domain, &principal) {
                return StatusCode::FORBIDDEN.into_response();
            }
            Json(serde_json::json!({
                "content": domain.northstar_md,
                "version": domain.northstar_version,
                "modified_by": domain.northstar_modified_by,
                "modified_at": domain.northstar_modified_at,
            }))
            .into_response()
        }
        Ok(None) => not_found("Domain not found"),
        Err(e) => {
            tracing::error!("get_domain_northstar: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn put_domain_northstar_handler(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
    Json(payload): Json<NorthstarUpdate>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT * FROM domain WHERE id = $1")
        .bind(&domain_id)
        .fetch_optional(&state.db)
        .await;

    let domain = match row {
        Ok(Some(r)) => row_to_domain(&r),
        Ok(None) => return not_found("Domain not found"),
        Err(e) => {
            tracing::error!("put_domain_northstar fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if !can_write(&domain, &principal) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let result = sqlx::query(
        "UPDATE domain \
         SET northstar_md = $1, \
             northstar_version = northstar_version + 1, \
             northstar_modified_by = $2, \
             northstar_modified_at = NOW(), \
             updated_at = NOW() \
         WHERE id = $3 \
         RETURNING northstar_md, northstar_version, northstar_modified_by, northstar_modified_at",
    )
    .bind(&payload.content)
    .bind(&principal.subject)
    .bind(&domain_id)
    .fetch_optional(&state.db)
    .await;

    match result {
        Ok(Some(r)) => Json(serde_json::json!({
            "content": r.get::<String, _>("northstar_md"),
            "version": r.get::<i32, _>("northstar_version"),
            "modified_by": r.get::<String, _>("northstar_modified_by"),
            "modified_at": r.get::<Option<chrono::NaiveDateTime>, _>("northstar_modified_at"),
        }))
        .into_response(),
        Ok(None) => not_found("Domain not found"),
        Err(e) => {
            tracing::error!("put_domain_northstar update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
