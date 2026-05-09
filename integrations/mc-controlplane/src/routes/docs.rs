use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/docs", get(list_docs).post(create_doc))
        .route(
            "/docs/{doc_id}",
            get(get_doc).patch(update_doc).delete(delete_doc),
        )
        .route("/docs/{doc_id}/publish", axum::routing::post(publish_doc))
}

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(json!({"detail": msg}))).into_response()
}

fn row_to_doc(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    json!({
        "id": row.get::<i32, _>("id"),
        "kluster_id": row.get::<String, _>("kluster_id"),
        "title": row.get::<String, _>("title"),
        "body": row.get::<String, _>("body"),
        "doc_type": row.get::<String, _>("doc_type"),
        "status": row.get::<String, _>("status"),
        "version": row.get::<i32, _>("version"),
        "provenance": row.get::<String, _>("provenance"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

async fn can_read_mission(db: &sqlx::PgPool, principal: &Principal, mission_id: &str) -> bool {
    if principal.is_admin {
        return true;
    }
    if let Ok(Some(row)) = sqlx::query(
        "SELECT visibility, owners, contributors FROM mission WHERE id=$1",
    )
    .bind(mission_id)
    .fetch_optional(db)
    .await
    {
        let visibility: String = row.get("visibility");
        if visibility.to_lowercase() == "public" {
            return true;
        }
        let owners: String = row.get("owners");
        let contributors: String = row.get("contributors");
        let sub = principal.subject.to_lowercase();
        let in_list =
            |s: &str| s.split(',').map(|x| x.trim().to_lowercase()).any(|x| x == sub);
        return in_list(&owners) || in_list(&contributors);
    }
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM missionrolemembership WHERE mission_id=$1 AND subject=$2)",
    )
    .bind(mission_id)
    .bind(&principal.subject)
    .fetch_one(db)
    .await
    .unwrap_or(false)
}

async fn can_write_mission(db: &sqlx::PgPool, principal: &Principal, mission_id: &str) -> bool {
    if principal.is_admin {
        return true;
    }
    if let Ok(Some(row)) =
        sqlx::query("SELECT owners, contributors FROM mission WHERE id=$1")
            .bind(mission_id)
            .fetch_optional(db)
            .await
    {
        let owners: String = row.get("owners");
        let contributors: String = row.get("contributors");
        let sub = principal.subject.to_lowercase();
        let in_list =
            |s: &str| s.split(',').map(|x| x.trim().to_lowercase()).any(|x| x == sub);
        return in_list(&owners) || in_list(&contributors);
    }
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM missionrolemembership WHERE mission_id=$1 AND subject=$2 AND role IN ('owner','contributor'))",
    )
    .bind(mission_id)
    .bind(&principal.subject)
    .fetch_one(db)
    .await
    .unwrap_or(false)
}

#[derive(Deserialize)]
struct ListDocsQuery {
    kluster_id: Option<String>,
    limit: Option<i64>,
}

async fn create_doc(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let kluster_id = match payload.get("kluster_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"detail": "kluster_id is required"})),
            )
                .into_response()
        }
    };
    let title = payload
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let body = payload
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let doc_type = payload
        .get("doc_type")
        .and_then(|v| v.as_str())
        .unwrap_or("narrative")
        .to_string();
    let status = payload
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("draft")
        .to_string();
    let provenance = payload
        .get("provenance")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Check kluster exists
    let kluster_row = match sqlx::query(
        "SELECT id, mission_id FROM kluster WHERE id=$1",
    )
    .bind(&kluster_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Kluster not found"),
        Err(e) => {
            tracing::error!("create_doc fetch kluster: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mission_id: Option<String> = kluster_row.try_get("mission_id").ok().and_then(|v: Option<String>| v);
    if let Some(ref mid) = mission_id {
        if !can_write_mission(&state.db, &principal, mid).await {
            return StatusCode::FORBIDDEN.into_response();
        }
    }

    let now = Utc::now().naive_utc();
    match sqlx::query(
        r#"INSERT INTO doc
            (kluster_id, title, body, doc_type, status, version, provenance, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,1,$6,$7,$7) RETURNING *"#,
    )
    .bind(&kluster_id)
    .bind(&title)
    .bind(&body)
    .bind(&doc_type)
    .bind(&status)
    .bind(&provenance)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => (StatusCode::OK, Json(row_to_doc(&row))).into_response(),
        Err(e) => {
            tracing::error!("create_doc insert: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_docs(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<ListDocsQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);

    let rows = if let Some(ref kid) = q.kluster_id {
        sqlx::query("SELECT * FROM doc WHERE kluster_id=$1 ORDER BY updated_at DESC LIMIT $2")
            .bind(kid)
            .bind(limit)
            .fetch_all(&state.db)
            .await
    } else {
        sqlx::query("SELECT * FROM doc ORDER BY updated_at DESC LIMIT $1")
            .bind(limit)
            .fetch_all(&state.db)
            .await
    };

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("list_docs query: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if principal.is_admin {
        let docs: Vec<serde_json::Value> = rows.iter().map(row_to_doc).collect();
        return Json(docs).into_response();
    }

    // Collect unique kluster_ids
    let kluster_ids: Vec<String> = rows
        .iter()
        .map(|r| r.get::<String, _>("kluster_id"))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    if kluster_ids.is_empty() {
        return Json(serde_json::Value::Array(vec![])).into_response();
    }

    // Fetch klusters to get mission_ids
    let placeholders: String = kluster_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("${}", i + 1))
        .collect::<Vec<_>>()
        .join(",");
    let query_str = format!(
        "SELECT id, mission_id FROM kluster WHERE id IN ({})",
        placeholders
    );
    let mut q_builder = sqlx::query(&query_str);
    for kid in &kluster_ids {
        q_builder = q_builder.bind(kid);
    }
    let kluster_rows = match q_builder.fetch_all(&state.db).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("list_docs fetch klusters: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Build kluster_id -> mission_id map and collect mission_ids
    let mut mission_by_kluster: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    for kr in &kluster_rows {
        let kid: String = kr.get("id");
        let mid: Option<String> = kr.try_get("mission_id").ok().and_then(|v: Option<String>| v);
        mission_by_kluster.insert(kid, mid);
    }

    // Check readable missions
    let mission_ids: Vec<String> = mission_by_kluster
        .values()
        .filter_map(|v| v.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let mut readable_missions: std::collections::HashSet<String> = std::collections::HashSet::new();
    for mid in &mission_ids {
        if can_read_mission(&state.db, &principal, mid).await {
            readable_missions.insert(mid.clone());
        }
    }

    let docs: Vec<serde_json::Value> = rows
        .iter()
        .filter(|r| {
            let kid: String = r.get("kluster_id");
            if let Some(Some(mid)) = mission_by_kluster.get(&kid) {
                readable_missions.contains(mid.as_str())
            } else {
                false
            }
        })
        .map(row_to_doc)
        .collect();

    Json(docs).into_response()
}

async fn get_doc(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(doc_id): Path<i32>,
) -> impl IntoResponse {
    let doc_row = match sqlx::query("SELECT * FROM doc WHERE id=$1")
        .bind(doc_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Doc not found"),
        Err(e) => {
            tracing::error!("get_doc: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let kluster_id: String = doc_row.get("kluster_id");
    let kluster_row = match sqlx::query("SELECT id, mission_id FROM kluster WHERE id=$1")
        .bind(&kluster_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Kluster not found"),
        Err(e) => {
            tracing::error!("get_doc fetch kluster: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mission_id: Option<String> = kluster_row.try_get("mission_id").ok().and_then(|v: Option<String>| v);
    match mission_id {
        Some(ref mid) => {
            if !can_read_mission(&state.db, &principal, mid).await {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
        None => {
            if !principal.is_admin {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }

    Json(row_to_doc(&doc_row)).into_response()
}

async fn update_doc(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(doc_id): Path<i32>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let doc_row = match sqlx::query("SELECT * FROM doc WHERE id=$1")
        .bind(doc_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Doc not found"),
        Err(e) => {
            tracing::error!("update_doc fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let kluster_id: String = doc_row.get("kluster_id");
    let kluster_row = match sqlx::query("SELECT id, mission_id FROM kluster WHERE id=$1")
        .bind(&kluster_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Kluster not found"),
        Err(e) => {
            tracing::error!("update_doc fetch kluster: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mission_id: Option<String> = kluster_row.try_get("mission_id").ok().and_then(|v: Option<String>| v);
    if let Some(ref mid) = mission_id {
        if !can_write_mission(&state.db, &principal, mid).await {
            return StatusCode::FORBIDDEN.into_response();
        }
    }

    // Merge fields
    let title = payload
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| doc_row.get::<String, _>("title"));
    let body = payload
        .get("body")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| doc_row.get::<String, _>("body"));
    let doc_type = payload
        .get("doc_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| doc_row.get::<String, _>("doc_type"));
    let status = payload
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| doc_row.get::<String, _>("status"));
    let provenance = payload
        .get("provenance")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| doc_row.get::<String, _>("provenance"));

    let current_version: i32 = doc_row.get("version");
    let now = Utc::now().naive_utc();

    match sqlx::query(
        "UPDATE doc SET title=$2, body=$3, doc_type=$4, status=$5, provenance=$6, \
         version=$7, updated_at=$8 WHERE id=$1 RETURNING *",
    )
    .bind(doc_id)
    .bind(&title)
    .bind(&body)
    .bind(&doc_type)
    .bind(&status)
    .bind(&provenance)
    .bind(current_version + 1)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => (StatusCode::OK, Json(row_to_doc(&row))).into_response(),
        Err(e) => {
            tracing::error!("update_doc: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn publish_doc(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(doc_id): Path<i32>,
) -> impl IntoResponse {
    let doc_row = match sqlx::query("SELECT * FROM doc WHERE id=$1")
        .bind(doc_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Doc not found"),
        Err(e) => {
            tracing::error!("publish_doc fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let kluster_id: String = doc_row.get("kluster_id");
    let kluster_row = match sqlx::query("SELECT id, mission_id FROM kluster WHERE id=$1")
        .bind(&kluster_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Kluster not found"),
        Err(e) => {
            tracing::error!("publish_doc fetch kluster: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mission_id: Option<String> = kluster_row.try_get("mission_id").ok().and_then(|v: Option<String>| v);
    if let Some(ref mid) = mission_id {
        if !can_write_mission(&state.db, &principal, mid).await {
            return StatusCode::FORBIDDEN.into_response();
        }
    }

    let current_version: i32 = doc_row.get("version");
    let now = Utc::now().naive_utc();

    match sqlx::query(
        "UPDATE doc SET status='published', version=$2, updated_at=$3 WHERE id=$1 RETURNING *",
    )
    .bind(doc_id)
    .bind(current_version + 1)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => (StatusCode::OK, Json(row_to_doc(&row))).into_response(),
        Err(e) => {
            tracing::error!("publish_doc update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn delete_doc(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(doc_id): Path<i32>,
) -> impl IntoResponse {
    let doc_row = match sqlx::query("SELECT * FROM doc WHERE id=$1")
        .bind(doc_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Doc not found"),
        Err(e) => {
            tracing::error!("delete_doc fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let kluster_id: String = doc_row.get("kluster_id");
    let kluster_row = match sqlx::query("SELECT id, mission_id FROM kluster WHERE id=$1")
        .bind(&kluster_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Kluster not found"),
        Err(e) => {
            tracing::error!("delete_doc fetch kluster: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mission_id: Option<String> = kluster_row.try_get("mission_id").ok().and_then(|v: Option<String>| v);
    match mission_id {
        Some(ref mid) => {
            if !can_write_mission(&state.db, &principal, mid).await {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
        None => {
            if !principal.is_admin {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }

    match sqlx::query("DELETE FROM doc WHERE id=$1")
        .bind(doc_id)
        .execute(&state.db)
        .await
    {
        Ok(_) => Json(json!({"ok": true, "deleted_id": doc_id})).into_response(),
        Err(e) => {
            tracing::error!("delete_doc: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
