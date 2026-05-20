use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{NaiveDateTime, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/me/profiles", get(list_profiles).post(create_profile))
        .route(
            "/me/profiles/{name}",
            get(get_profile).put(replace_profile).patch(patch_profile).delete(delete_profile),
        )
        .route("/me/profiles/{name}/download", get(download_profile))
        .route("/me/profiles/{name}/activate", post(activate_profile))
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn conflict(msg: &str) -> axum::response::Response {
    (StatusCode::CONFLICT, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn bad_request(msg: &str) -> axum::response::Response {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn row_to_profile(row: &sqlx::postgres::PgRow, include_tarball: bool) -> serde_json::Value {
    let manifest_json_str: String = row.try_get("manifest_json").unwrap_or_default();
    let manifest: serde_json::Value = serde_json::from_str(&manifest_json_str)
        .unwrap_or(serde_json::json!([]));

    let mut v = serde_json::json!({
        "id": row.get::<i32, _>("id"),
        "name": row.get::<String, _>("name"),
        "owner_subject": row.get::<String, _>("owner_subject"),
        "description": row.get::<String, _>("description"),
        "is_default": row.get::<bool, _>("is_default"),
        "manifest": manifest,
        "mirror_uri": row.get::<String, _>("mirror_uri"),
        "mirror_sha256": row.get::<String, _>("mirror_sha256"),
        "mirror_size_bytes": row.get::<i32, _>("mirror_size_bytes"),
        "mirrored_at": row.try_get::<Option<NaiveDateTime>, _>("mirrored_at").ok().flatten(),
        "sha256": row.try_get::<Option<String>, _>("sha256").ok().flatten(),
        "size_bytes": row.get::<i32, _>("size_bytes"),
        "created_at": row.get::<NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<NaiveDateTime, _>("updated_at"),
    });

    if include_tarball {
        let tarball: Option<String> = row.try_get("tarball_b64").ok().flatten();
        v["tarball_b64"] = serde_json::Value::String(tarball.unwrap_or_default());
    }

    v
}

fn validate_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 63 {
        return false;
    }
    let mut chars = name.chars();
    // First char: a-z or 0-9
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    // Remaining chars: a-z, 0-9, _, -
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn compute_sha256_and_size(tarball_b64: &str) -> Result<(String, i32), String> {
    let bytes = STANDARD
        .decode(tarball_b64)
        .map_err(|e| format!("Invalid base64: {e}"))?;
    let hash = hex::encode(Sha256::digest(&bytes));
    let size = bytes.len() as i32;
    Ok((hash, size))
}

// ── query structs ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LimitQuery {
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct CreateProfileBody {
    name: String,
    description: Option<String>,
    is_default: bool,
    manifest: Option<serde_json::Value>,
    tarball_b64: String,
    expected_sha256: Option<String>,
}

#[derive(Deserialize)]
struct PatchProfileBody {
    description: Option<String>,
    is_default: Option<bool>,
    manifest: Option<serde_json::Value>,
    tarball_b64: Option<String>,
    expected_sha256: Option<String>,
}

// ── handlers ──────────────────────────────────────────────────────────────────

async fn list_profiles(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<LimitQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);
    match sqlx::query(
        "SELECT * FROM userprofile WHERE owner_subject=$1 ORDER BY updated_at DESC LIMIT $2",
    )
    .bind(&principal.subject)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => Json(
            rows.iter()
                .map(|r| row_to_profile(r, false))
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => {
            tracing::error!("list_profiles: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn create_profile(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<CreateProfileBody>,
) -> impl IntoResponse {
    if !validate_name(&body.name) {
        return bad_request("Invalid profile name. Must match ^[a-z0-9][a-z0-9_-]{0,62}$");
    }

    let (sha256, size_bytes) = match compute_sha256_and_size(&body.tarball_b64) {
        Ok(v) => v,
        Err(e) => return bad_request(&e),
    };

    // Check name uniqueness for this owner
    let exists: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM userprofile WHERE owner_subject=$1 AND name=$2",
    )
    .bind(&principal.subject)
    .bind(&body.name)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    if exists.is_some() {
        return conflict("A profile with this name already exists");
    }

    let description = body.description.unwrap_or_default();
    let manifest_json = match &body.manifest {
        Some(m) => serde_json::to_string(m).unwrap_or_else(|_| "[]".to_string()),
        None => "[]".to_string(),
    };
    let now = Utc::now().naive_utc();

    // Clear is_default on others if needed
    if body.is_default {
        let _ = sqlx::query(
            "UPDATE userprofile SET is_default=false, updated_at=$2 WHERE owner_subject=$1 AND is_default=true",
        )
        .bind(&principal.subject)
        .bind(now)
        .execute(&state.db)
        .await;
    }

    let result = sqlx::query(
        "INSERT INTO userprofile \
         (name, owner_subject, description, is_default, manifest_json, tarball_b64, \
          mirror_uri, mirror_sha256, mirror_size_bytes, sha256, size_bytes, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,'','',0,$7,$8,$9,$9) RETURNING *",
    )
    .bind(&body.name)
    .bind(&principal.subject)
    .bind(&description)
    .bind(body.is_default)
    .bind(&manifest_json)
    .bind(&body.tarball_b64)
    .bind(&sha256)
    .bind(size_bytes)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => (StatusCode::CREATED, Json(row_to_profile(&row, false))).into_response(),
        Err(e) if e.to_string().contains("unique") || e.to_string().contains("duplicate") => {
            conflict("A profile with this name already exists")
        }
        Err(e) => {
            tracing::error!("create_profile: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_profile(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match sqlx::query(
        "SELECT * FROM userprofile WHERE owner_subject=$1 AND name=$2",
    )
    .bind(&principal.subject)
    .bind(&name)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => Json(row_to_profile(&row, false)).into_response(),
        Ok(None) => not_found("Profile not found"),
        Err(e) => {
            tracing::error!("get_profile: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn replace_profile(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(name): Path<String>,
    Json(body): Json<CreateProfileBody>,
) -> impl IntoResponse {
    // Verify ownership
    let existing = sqlx::query(
        "SELECT * FROM userprofile WHERE owner_subject=$1 AND name=$2",
    )
    .bind(&principal.subject)
    .bind(&name)
    .fetch_optional(&state.db)
    .await;

    match existing {
        Ok(None) => return not_found("Profile not found"),
        Err(e) => {
            tracing::error!("replace_profile fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(Some(_)) => {}
    }

    let (sha256, size_bytes) = match compute_sha256_and_size(&body.tarball_b64) {
        Ok(v) => v,
        Err(e) => return bad_request(&e),
    };

    // Check expected_sha256 if provided
    if let Some(expected) = &body.expected_sha256 {
        if *expected != sha256 {
            return conflict("SHA256 mismatch: tarball does not match expected_sha256");
        }
    }

    let description = body.description.unwrap_or_default();
    let manifest_json = match &body.manifest {
        Some(m) => serde_json::to_string(m).unwrap_or_else(|_| "[]".to_string()),
        None => "[]".to_string(),
    };
    let now = Utc::now().naive_utc();

    if body.is_default {
        let _ = sqlx::query(
            "UPDATE userprofile SET is_default=false, updated_at=$2 \
             WHERE owner_subject=$1 AND is_default=true AND name!=$3",
        )
        .bind(&principal.subject)
        .bind(now)
        .bind(&name)
        .execute(&state.db)
        .await;
    }

    let result = sqlx::query(
        "UPDATE userprofile SET description=$3, is_default=$4, manifest_json=$5, tarball_b64=$6, \
         sha256=$7, size_bytes=$8, updated_at=$9 \
         WHERE owner_subject=$1 AND name=$2 RETURNING *",
    )
    .bind(&principal.subject)
    .bind(&name)
    .bind(&description)
    .bind(body.is_default)
    .bind(&manifest_json)
    .bind(&body.tarball_b64)
    .bind(&sha256)
    .bind(size_bytes)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => Json(row_to_profile(&row, false)).into_response(),
        Err(e) => {
            tracing::error!("replace_profile: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn patch_profile(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(name): Path<String>,
    Json(body): Json<PatchProfileBody>,
) -> impl IntoResponse {
    let existing = sqlx::query(
        "SELECT * FROM userprofile WHERE owner_subject=$1 AND name=$2",
    )
    .bind(&principal.subject)
    .bind(&name)
    .fetch_optional(&state.db)
    .await;

    let row = match existing {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Profile not found"),
        Err(e) => {
            tracing::error!("patch_profile fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Resolve fields with fallbacks from existing row
    let description: String = body
        .description
        .unwrap_or_else(|| row.get::<String, _>("description"));

    let is_default: bool = body
        .is_default
        .unwrap_or_else(|| row.get::<bool, _>("is_default"));

    let manifest_json: String = match body.manifest {
        Some(m) => serde_json::to_string(&m).unwrap_or_else(|_| "[]".to_string()),
        None => row.try_get("manifest_json").unwrap_or_default(),
    };

    let now = Utc::now().naive_utc();

    let (tarball_b64, sha256, size_bytes) = if let Some(tb) = &body.tarball_b64 {
        match compute_sha256_and_size(tb) {
            Ok((h, s)) => {
                // Check expected_sha256 if provided
                if let Some(expected) = &body.expected_sha256 {
                    if *expected != h {
                        return conflict("SHA256 mismatch: tarball does not match expected_sha256");
                    }
                }
                (tb.clone(), h, s)
            }
            Err(e) => return bad_request(&e),
        }
    } else {
        let existing_tarball: Option<String> = row.try_get("tarball_b64").ok().flatten();
        let existing_sha256: Option<String> = row.try_get("sha256").ok().flatten();
        let existing_size: i32 = row.try_get("size_bytes").unwrap_or(0);
        (
            existing_tarball.unwrap_or_default(),
            existing_sha256.unwrap_or_default(),
            existing_size,
        )
    };

    if is_default {
        let _ = sqlx::query(
            "UPDATE userprofile SET is_default=false, updated_at=$2 \
             WHERE owner_subject=$1 AND is_default=true AND name!=$3",
        )
        .bind(&principal.subject)
        .bind(now)
        .bind(&name)
        .execute(&state.db)
        .await;
    }

    let result = sqlx::query(
        "UPDATE userprofile SET description=$3, is_default=$4, manifest_json=$5, tarball_b64=$6, \
         sha256=$7, size_bytes=$8, updated_at=$9 \
         WHERE owner_subject=$1 AND name=$2 RETURNING *",
    )
    .bind(&principal.subject)
    .bind(&name)
    .bind(&description)
    .bind(is_default)
    .bind(&manifest_json)
    .bind(&tarball_b64)
    .bind(&sha256)
    .bind(size_bytes)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(updated) => Json(row_to_profile(&updated, false)).into_response(),
        Err(e) => {
            tracing::error!("patch_profile: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn delete_profile(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let result = sqlx::query(
        "DELETE FROM userprofile WHERE owner_subject=$1 AND name=$2",
    )
    .bind(&principal.subject)
    .bind(&name)
    .execute(&state.db)
    .await;

    match result {
        Ok(r) if r.rows_affected() == 0 => not_found("Profile not found"),
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!("delete_profile: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn download_profile(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match sqlx::query(
        "SELECT * FROM userprofile WHERE owner_subject=$1 AND name=$2",
    )
    .bind(&principal.subject)
    .bind(&name)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => {
            let tarball: Option<String> = row.try_get("tarball_b64").ok().flatten();
            if tarball.as_deref().unwrap_or("").is_empty() {
                return not_found("Profile has no tarball");
            }
            Json(row_to_profile(&row, true)).into_response()
        }
        Ok(None) => not_found("Profile not found"),
        Err(e) => {
            tracing::error!("download_profile: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn activate_profile(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(name): Path<String>,
) -> impl IntoResponse {
    // Verify profile exists and is owned
    let existing = sqlx::query(
        "SELECT id FROM userprofile WHERE owner_subject=$1 AND name=$2",
    )
    .bind(&principal.subject)
    .bind(&name)
    .fetch_optional(&state.db)
    .await;

    match existing {
        Ok(None) => return not_found("Profile not found"),
        Err(e) => {
            tracing::error!("activate_profile fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(Some(_)) => {}
    }

    let now = Utc::now().naive_utc();

    // Clear all is_default for this owner
    let _ = sqlx::query(
        "UPDATE userprofile SET is_default=false, updated_at=$2 WHERE owner_subject=$1",
    )
    .bind(&principal.subject)
    .bind(now)
    .execute(&state.db)
    .await;

    // Set this one as default
    let result = sqlx::query(
        "UPDATE userprofile SET is_default=true, updated_at=$3 \
         WHERE owner_subject=$1 AND name=$2 RETURNING *",
    )
    .bind(&principal.subject)
    .bind(&name)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => Json(row_to_profile(&row, false)).into_response(),
        Err(e) => {
            tracing::error!("activate_profile: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
