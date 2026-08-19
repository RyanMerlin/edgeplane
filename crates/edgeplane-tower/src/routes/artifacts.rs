use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::Engine;
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/artifacts", get(list_artifacts).post(create_artifact))
        .route(
            "/artifacts/{artifact_id}",
            get(get_artifact)
                .patch(update_artifact)
                .delete(delete_artifact),
        )
        .route(
            "/artifacts/{artifact_id}/content",
            get(get_artifact_content),
        )
        .route(
            "/artifacts/{artifact_id}/download-url",
            get(get_artifact_download_url),
        )
        .route(
            "/artifacts/{artifact_id}/publish",
            axum::routing::post(publish_artifact),
        )
}

fn not_found(msg: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"detail": msg}))).into_response()
}

fn row_to_artifact(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    json!({
        "id": row.get::<i32, _>("id"),
        "mission_id": row.get::<String, _>("mission_id"),
        "name": row.get::<String, _>("name"),
        "artifact_type": row.get::<String, _>("artifact_type"),
        "uri": row.get::<String, _>("uri"),
        "storage_backend": row.get::<String, _>("storage_backend"),
        "content_sha256": row.get::<String, _>("content_sha256"),
        "size_bytes": row.get::<i32, _>("size_bytes"),
        "mime_type": row.get::<String, _>("mime_type"),
        "storage_class": row.get::<String, _>("storage_class"),
        "content_b64": row.get::<Option<String>, _>("content_b64"),
        "external_pointer": row.get::<bool, _>("external_pointer"),
        "external_uri": row.get::<String, _>("external_uri"),
        "status": row.get::<String, _>("status"),
        "version": row.get::<i32, _>("version"),
        "provenance": row.get::<String, _>("provenance"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

async fn can_read_domain(db: &sqlx::PgPool, principal: &Principal, domain_id: &str) -> bool {
    if principal.is_admin {
        return true;
    }
    if let Ok(Some(row)) =
        sqlx::query("SELECT visibility, owners, contributors FROM domain WHERE id=$1")
            .bind(domain_id)
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
        let in_list = |s: &str| {
            s.split(',')
                .map(|x| x.trim().to_lowercase())
                .any(|x| x == sub)
        };
        return in_list(&owners) || in_list(&contributors);
    }
    false
}

async fn can_write_domain(db: &sqlx::PgPool, principal: &Principal, domain_id: &str) -> bool {
    if principal.is_admin {
        return true;
    }
    if let Ok(Some(row)) = sqlx::query("SELECT owners, contributors FROM domain WHERE id=$1")
        .bind(domain_id)
        .fetch_optional(db)
        .await
    {
        let owners: String = row.get("owners");
        let contributors: String = row.get("contributors");
        let sub = principal.subject.to_lowercase();
        let in_list = |s: &str| {
            s.split(',')
                .map(|x| x.trim().to_lowercase())
                .any(|x| x == sub)
        };
        return in_list(&owners) || in_list(&contributors);
    }
    false
}

#[derive(Deserialize)]
struct ListArtifactsQuery {
    mission_id: Option<String>,
    limit: Option<i64>,
}

async fn create_artifact(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let mission_id = match payload.get("mission_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"detail": "mission_id is required"})),
            )
                .into_response();
        }
    };

    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let artifact_type = payload
        .get("artifact_type")
        .and_then(|v| v.as_str())
        .unwrap_or("file")
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
    let mime_type = payload
        .get("mime_type")
        .and_then(|v| v.as_str())
        .unwrap_or("application/octet-stream")
        .to_string();

    let content_b64_raw = payload
        .get("content_b64")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let external_pointer_raw = payload
        .get("external_pointer")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let external_uri_raw = payload
        .get("external_uri")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let uri_raw = payload
        .get("uri")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let storage_backend_raw = payload
        .get("storage_backend")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let storage_class_raw = payload
        .get("storage_class")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Validate: content_b64 and external_pointer cannot both be set
    if !content_b64_raw.is_empty() && external_pointer_raw {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "content_b64 and external_pointer cannot both be set"})),
        )
            .into_response();
    }

    // Check mission exists
    let mission_row = match sqlx::query("SELECT id, domain_id FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Mission not found"),
        Err(e) => {
            tracing::error!("create_artifact fetch mission: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let domain_id: Option<String> = mission_row
        .try_get("domain_id")
        .ok()
        .and_then(|v: Option<String>| v);
    match domain_id {
        Some(ref mid) => {
            if !can_write_domain(&state.db, &principal, mid).await {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
        None => {
            if !principal.is_admin {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }

    // Resolve storage fields
    let (
        final_uri,
        final_storage_backend,
        final_storage_class,
        final_content_b64,
        final_external_pointer,
        final_external_uri,
        final_content_sha256,
        final_size_bytes,
    ) = if !content_b64_raw.is_empty() {
        // Decode and store inline (no S3 in Rust)
        let body = match base64::engine::general_purpose::STANDARD.decode(&content_b64_raw) {
            Ok(b) => b,
            Err(e) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({"detail": format!("Invalid content_b64: {}", e)})),
                )
                    .into_response();
            }
        };
        let sha256 = hex::encode(Sha256::digest(&body));
        let size = body.len() as i32;
        let uri = if uri_raw.is_empty() {
            format!("db-inline://artifacts/{}", sha256)
        } else {
            uri_raw.clone()
        };
        (
            uri,
            "inline".to_string(),
            "db_inline".to_string(),
            Some(content_b64_raw),
            false,
            String::new(),
            sha256,
            size,
        )
    } else if external_pointer_raw || !external_uri_raw.is_empty() {
        let resolved_uri = if !external_uri_raw.is_empty() {
            external_uri_raw.clone()
        } else {
            uri_raw.clone()
        };
        if resolved_uri.is_empty() {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"detail": "external pointer mode requires external_uri or uri"})),
            )
                .into_response();
        }
        (
            resolved_uri.clone(),
            "external".to_string(),
            "external_pointer".to_string(),
            None,
            true,
            resolved_uri,
            String::new(),
            0i32,
        )
    } else {
        // Use provided values as-is
        let backend = if !storage_backend_raw.is_empty() {
            storage_backend_raw
        } else if uri_raw.starts_with("s3://") {
            "s3".to_string()
        } else {
            String::new()
        };
        let class = storage_class_raw;
        let sha256 = payload
            .get("content_sha256")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let size = payload
            .get("size_bytes")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        (
            uri_raw,
            backend,
            class,
            None::<String>,
            false,
            external_uri_raw,
            sha256,
            size,
        )
    };

    let now = Utc::now().naive_utc();
    match sqlx::query(
        r#"INSERT INTO artifact
            (mission_id, name, artifact_type, uri, storage_backend, content_sha256, size_bytes,
             mime_type, storage_class, content_b64, external_pointer, external_uri,
             status, version, provenance, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,1,$14,$15,$15) RETURNING *"#,
    )
    .bind(&mission_id)
    .bind(&name)
    .bind(&artifact_type)
    .bind(&final_uri)
    .bind(&final_storage_backend)
    .bind(&final_content_sha256)
    .bind(final_size_bytes)
    .bind(&mime_type)
    .bind(&final_storage_class)
    .bind(&final_content_b64)
    .bind(final_external_pointer)
    .bind(&final_external_uri)
    .bind(&status)
    .bind(&provenance)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => (StatusCode::OK, Json(row_to_artifact(&row))).into_response(),
        Err(e) => {
            tracing::error!("create_artifact insert: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_artifacts(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<ListArtifactsQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);

    let rows = if let Some(ref kid) = q.mission_id {
        sqlx::query("SELECT * FROM artifact WHERE mission_id=$1 ORDER BY updated_at DESC LIMIT $2")
            .bind(kid)
            .bind(limit)
            .fetch_all(&state.db)
            .await
    } else {
        sqlx::query("SELECT * FROM artifact ORDER BY updated_at DESC LIMIT $1")
            .bind(limit)
            .fetch_all(&state.db)
            .await
    };

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("list_artifacts query: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if principal.is_admin {
        let artifacts: Vec<serde_json::Value> = rows.iter().map(row_to_artifact).collect();
        return Json(artifacts).into_response();
    }

    // Collect unique mission_ids
    let mission_ids: Vec<String> = rows
        .iter()
        .map(|r| r.get::<String, _>("mission_id"))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    if mission_ids.is_empty() {
        return Json(serde_json::Value::Array(vec![])).into_response();
    }

    let placeholders: String = mission_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("${}", i + 1))
        .collect::<Vec<_>>()
        .join(",");
    let query_str = format!(
        "SELECT id, domain_id FROM mission WHERE id IN ({})",
        placeholders
    );
    let mut q_builder = sqlx::query(&query_str);
    for kid in &mission_ids {
        q_builder = q_builder.bind(kid);
    }
    let mission_rows = match q_builder.fetch_all(&state.db).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("list_artifacts fetch missions: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mut domain_by_mission: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    for kr in &mission_rows {
        let kid: String = kr.get("id");
        let mid: Option<String> = kr.try_get("domain_id").ok().and_then(|v: Option<String>| v);
        domain_by_mission.insert(kid, mid);
    }

    let domain_ids: Vec<String> = domain_by_mission
        .values()
        .filter_map(|v| v.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let mut readable_domains: std::collections::HashSet<String> = std::collections::HashSet::new();
    for mid in &domain_ids {
        if can_read_domain(&state.db, &principal, mid).await {
            readable_domains.insert(mid.clone());
        }
    }

    let artifacts: Vec<serde_json::Value> = rows
        .iter()
        .filter(|r| {
            let kid: String = r.get("mission_id");
            if let Some(Some(mid)) = domain_by_mission.get(&kid) {
                readable_domains.contains(mid.as_str())
            } else {
                false
            }
        })
        .map(row_to_artifact)
        .collect();

    Json(artifacts).into_response()
}

async fn get_artifact(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(artifact_id): Path<i32>,
) -> impl IntoResponse {
    let artifact_row = match sqlx::query("SELECT * FROM artifact WHERE id=$1")
        .bind(artifact_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Artifact not found"),
        Err(e) => {
            tracing::error!("get_artifact: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mission_id: String = artifact_row.get("mission_id");
    let mission_row = match sqlx::query("SELECT id, domain_id FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Mission not found"),
        Err(e) => {
            tracing::error!("get_artifact fetch mission: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let domain_id: Option<String> = mission_row
        .try_get("domain_id")
        .ok()
        .and_then(|v: Option<String>| v);
    match domain_id {
        Some(ref mid) => {
            if !can_read_domain(&state.db, &principal, mid).await {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
        None => {
            if !principal.is_admin {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }

    Json(row_to_artifact(&artifact_row)).into_response()
}

async fn get_artifact_content(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(artifact_id): Path<i32>,
) -> impl IntoResponse {
    let artifact_row = match sqlx::query("SELECT * FROM artifact WHERE id=$1")
        .bind(artifact_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Artifact not found"),
        Err(e) => {
            tracing::error!("get_artifact_content: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mission_id: String = artifact_row.get("mission_id");
    let mission_row = match sqlx::query("SELECT id, domain_id FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Mission not found"),
        Err(e) => {
            tracing::error!("get_artifact_content fetch mission: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let domain_id: Option<String> = mission_row
        .try_get("domain_id")
        .ok()
        .and_then(|v: Option<String>| v);
    match domain_id {
        Some(ref mid) => {
            if !can_read_domain(&state.db, &principal, mid).await {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
        None => {
            if !principal.is_admin {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }

    let storage_backend: String = artifact_row.get("storage_backend");
    if storage_backend == "s3" {
        return (
            StatusCode::CONFLICT,
            Json(json!({"detail": "S3 content not available through this node"})),
        )
            .into_response();
    }

    let content_b64: Option<String> = artifact_row.get("content_b64");
    if let Some(b64) = content_b64 {
        match base64::engine::general_purpose::STANDARD.decode(&b64) {
            Ok(body_bytes) => {
                let mime: String = artifact_row.get("mime_type");
                let mime_val = if mime.is_empty() {
                    "application/octet-stream".to_string()
                } else {
                    mime
                };
                // mime_val is stored verbatim from the upload payload and is not
                // guaranteed to be a valid HTTP header value (e.g. it could contain
                // a newline or other control character). Never trust it enough to
                // unwrap header/response construction on user input — fall back to
                // a safe default instead of panicking the handler.
                let content_type = header::HeaderValue::from_str(&mime_val).unwrap_or_else(|_| {
                    header::HeaderValue::from_static("application/octet-stream")
                });
                return match Response::builder()
                    .header(header::CONTENT_TYPE, content_type)
                    .body(axum::body::Body::from(body_bytes))
                {
                    Ok(resp) => resp.into_response(),
                    Err(e) => {
                        tracing::error!("get_artifact_content build response: {e}");
                        StatusCode::INTERNAL_SERVER_ERROR.into_response()
                    }
                };
            }
            Err(e) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({"detail": format!("Invalid inline artifact content: {}", e)})),
                )
                    .into_response();
            }
        }
    }

    (
        StatusCode::CONFLICT,
        Json(json!({"detail": "Artifact does not have retrievable managed content"})),
    )
        .into_response()
}

async fn get_artifact_download_url(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(artifact_id): Path<i32>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT * FROM artifact WHERE id=$1")
        .bind(artifact_id)
        .fetch_optional(&state.db)
        .await;
    let row = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"detail": "Artifact not found"})),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("get_artifact_download_url: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mission_id: String = row.get("mission_id");
    let domain_id: Option<String> = sqlx::query_scalar("SELECT domain_id FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None)
        .flatten();
    if let Some(ref mid) = domain_id {
        if !can_read_domain(&state.db, &principal, mid).await {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"detail": "Forbidden"})),
            )
                .into_response();
        }
    } else if !principal.is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "Forbidden"})),
        )
            .into_response();
    }

    let storage_backend: String = row.try_get("storage_backend").unwrap_or_default();
    let uri: String = row.try_get("uri").unwrap_or_default();
    let expires_seconds: i64 = q
        .get("expires_seconds")
        .and_then(|v| v.parse().ok())
        .unwrap_or(60)
        .clamp(1, 3600);

    if storage_backend == "s3" {
        // S3 presigning requires AWS SDK — not available in edgeplane-tower; return 409 with the URI
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "detail": "S3 presigning requires the Python API",
                "artifact_id": artifact_id,
                "uri": uri,
            })),
        )
            .into_response();
    }

    Json(serde_json::json!({
        "artifact_id": artifact_id,
        "uri": uri,
        "expires_seconds": expires_seconds,
        "download_url": uri,
    }))
    .into_response()
}

async fn update_artifact(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(artifact_id): Path<i32>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let artifact_row = match sqlx::query("SELECT * FROM artifact WHERE id=$1")
        .bind(artifact_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Artifact not found"),
        Err(e) => {
            tracing::error!("update_artifact fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mission_id: String = artifact_row.get("mission_id");
    let mission_row = match sqlx::query("SELECT id, domain_id FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Mission not found"),
        Err(e) => {
            tracing::error!("update_artifact fetch mission: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let domain_id: Option<String> = mission_row
        .try_get("domain_id")
        .ok()
        .and_then(|v: Option<String>| v);
    match domain_id {
        Some(ref mid) => {
            if !can_write_domain(&state.db, &principal, mid).await {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
        None => {
            if !principal.is_admin {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }

    // Merge updatable fields
    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| artifact_row.get::<String, _>("name"));
    let artifact_type = payload
        .get("artifact_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| artifact_row.get::<String, _>("artifact_type"));
    let uri = payload
        .get("uri")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| artifact_row.get::<String, _>("uri"));
    let status = payload
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| artifact_row.get::<String, _>("status"));
    let provenance = payload
        .get("provenance")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| artifact_row.get::<String, _>("provenance"));
    let storage_backend = payload
        .get("storage_backend")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| artifact_row.get::<String, _>("storage_backend"));
    let storage_class = payload
        .get("storage_class")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| artifact_row.get::<String, _>("storage_class"));
    let mime_type = payload
        .get("mime_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| artifact_row.get::<String, _>("mime_type"));
    let external_pointer = payload
        .get("external_pointer")
        .and_then(|v| v.as_bool())
        .unwrap_or_else(|| artifact_row.get::<bool, _>("external_pointer"));
    let external_uri = payload
        .get("external_uri")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| artifact_row.get::<String, _>("external_uri"));

    // Handle content_b64 update — always store inline (no S3 in Rust)
    let content_b64_update = payload
        .get("content_b64")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let (
        final_uri,
        final_storage_backend,
        final_storage_class,
        final_content_b64,
        final_content_sha256,
        final_size_bytes,
    ) = if let Some(b64) = content_b64_update {
        match base64::engine::general_purpose::STANDARD.decode(&b64) {
            Ok(body) => {
                let sha256 = hex::encode(Sha256::digest(&body));
                let size = body.len() as i32;
                let final_uri_val = if uri.is_empty() {
                    format!("db-inline://artifacts/{}", sha256)
                } else {
                    uri.clone()
                };
                (
                    final_uri_val,
                    "inline".to_string(),
                    "db_inline".to_string(),
                    Some(b64),
                    sha256,
                    size,
                )
            }
            Err(e) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({"detail": format!("Invalid content_b64: {}", e)})),
                )
                    .into_response();
            }
        }
    } else {
        let sha256 = payload
            .get("content_sha256")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| artifact_row.get::<String, _>("content_sha256"));
        let size = payload
            .get("size_bytes")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32)
            .unwrap_or_else(|| artifact_row.get::<i32, _>("size_bytes"));
        let existing_b64: Option<String> = artifact_row.get("content_b64");
        (
            uri,
            storage_backend,
            storage_class,
            existing_b64,
            sha256,
            size,
        )
    };

    let current_version: i32 = artifact_row.get("version");
    let now = Utc::now().naive_utc();

    match sqlx::query(
        "UPDATE artifact SET name=$2, artifact_type=$3, uri=$4, storage_backend=$5, \
         content_sha256=$6, size_bytes=$7, mime_type=$8, storage_class=$9, content_b64=$10, \
         external_pointer=$11, external_uri=$12, status=$13, provenance=$14, \
         version=$15, updated_at=$16 WHERE id=$1 RETURNING *",
    )
    .bind(artifact_id)
    .bind(&name)
    .bind(&artifact_type)
    .bind(&final_uri)
    .bind(&final_storage_backend)
    .bind(&final_content_sha256)
    .bind(final_size_bytes)
    .bind(&mime_type)
    .bind(&final_storage_class)
    .bind(&final_content_b64)
    .bind(external_pointer)
    .bind(&external_uri)
    .bind(&status)
    .bind(&provenance)
    .bind(current_version + 1)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => (StatusCode::OK, Json(row_to_artifact(&row))).into_response(),
        Err(e) => {
            tracing::error!("update_artifact: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn publish_artifact(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(artifact_id): Path<i32>,
) -> impl IntoResponse {
    let artifact_row = match sqlx::query("SELECT * FROM artifact WHERE id=$1")
        .bind(artifact_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Artifact not found"),
        Err(e) => {
            tracing::error!("publish_artifact fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mission_id: String = artifact_row.get("mission_id");
    let mission_row = match sqlx::query("SELECT id, domain_id FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Mission not found"),
        Err(e) => {
            tracing::error!("publish_artifact fetch mission: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let domain_id: Option<String> = mission_row
        .try_get("domain_id")
        .ok()
        .and_then(|v: Option<String>| v);
    match domain_id {
        Some(ref mid) => {
            if !can_write_domain(&state.db, &principal, mid).await {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
        None => {
            if !principal.is_admin {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }

    let current_version: i32 = artifact_row.get("version");
    let now = Utc::now().naive_utc();

    match sqlx::query(
        "UPDATE artifact SET status='published', version=$2, updated_at=$3 WHERE id=$1 RETURNING *",
    )
    .bind(artifact_id)
    .bind(current_version + 1)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => (StatusCode::OK, Json(row_to_artifact(&row))).into_response(),
        Err(e) => {
            tracing::error!("publish_artifact update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn delete_artifact(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(artifact_id): Path<i32>,
) -> impl IntoResponse {
    let artifact_row = match sqlx::query("SELECT * FROM artifact WHERE id=$1")
        .bind(artifact_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Artifact not found"),
        Err(e) => {
            tracing::error!("delete_artifact fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mission_id: String = artifact_row.get("mission_id");
    let mission_row = match sqlx::query("SELECT id, domain_id FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Mission not found"),
        Err(e) => {
            tracing::error!("delete_artifact fetch mission: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let domain_id: Option<String> = mission_row
        .try_get("domain_id")
        .ok()
        .and_then(|v: Option<String>| v);
    match domain_id {
        Some(ref mid) => {
            if !can_write_domain(&state.db, &principal, mid).await {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
        None => {
            if !principal.is_admin {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }

    match sqlx::query("DELETE FROM artifact WHERE id=$1")
        .bind(artifact_id)
        .execute(&state.db)
        .await
    {
        Ok(_) => Json(json!({"ok": true, "deleted_id": artifact_id})).into_response(),
        Err(e) => {
            tracing::error!("delete_artifact: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
