use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use base64::Engine;
use chrono::Utc;
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/domains/{domain_id}/skills/bundles",
            post(create_domain_bundle),
        )
        .route(
            "/domains/{domain_id}/missions/{mission_id}/skills/bundles",
            post(create_mission_bundle),
        )
        .route(
            "/domains/{domain_id}/skills/bundles/{bundle_id}/deprecate",
            post(deprecate_bundle),
        )
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

fn not_found(msg: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn unprocessable(msg: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(serde_json::json!({"detail": msg})),
    )
        .into_response()
}

fn forbidden() -> Response {
    StatusCode::FORBIDDEN.into_response()
}

// ---------------------------------------------------------------------------
// Crypto / encoding helpers
// ---------------------------------------------------------------------------

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(data))
}

fn new_hash_id() -> String {
    let bytes: [u8; 6] = rand::random();
    hex::encode(bytes)
}

fn canon_json(v: &serde_json::Value) -> String {
    fn sort_value(v: &serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::Object(map) => {
                let mut sorted: Vec<(String, serde_json::Value)> =
                    map.iter().map(|(k, v)| (k.clone(), sort_value(v))).collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                serde_json::Value::Object(sorted.into_iter().collect())
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(sort_value).collect())
            }
            other => other.clone(),
        }
    }
    serde_json::to_string(&sort_value(v)).unwrap_or_default()
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn decode_tarball_b64(b64: &str) -> Result<Vec<u8>, Response> {
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| unprocessable(&format!("Invalid tarball_b64: {e}")))
}

fn extract_tar_entries(
    tarball_bytes: &[u8],
) -> Result<std::collections::BTreeMap<String, Vec<u8>>, Response> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    use tar::Archive;

    let gz = GzDecoder::new(tarball_bytes);
    let mut archive = Archive::new(gz);
    let mut entries: std::collections::BTreeMap<String, Vec<u8>> = Default::default();

    for entry_result in archive
        .entries()
        .map_err(|e| unprocessable(&format!("Invalid tar.gz: {e}")))?
    {
        let mut entry =
            entry_result.map_err(|e| unprocessable(&format!("Tar read error: {e}")))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|e| unprocessable(&format!("Bad tar path: {e}")))?
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_string();
        if path.is_empty() || path.starts_with("../") || path.contains("/../") {
            return Err(unprocessable(&format!("Invalid tar member path: {path}")));
        }
        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .map_err(|e| unprocessable(&format!("Tar read error: {e}")))?;
        entries.insert(path, data);
    }

    if entries.is_empty() {
        return Err(unprocessable("Skill bundle tarball has no files"));
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------
// Manifest builder
// ---------------------------------------------------------------------------

fn build_normalized_manifest(
    scope_type: &str,
    scope_id: &str,
    domain_id: &str,
    mission_id: &str,
    manifest_payload: &serde_json::Value,
    entries: &std::collections::BTreeMap<String, Vec<u8>>,
) -> Result<serde_json::Value, Response> {
    // Build file-to-listed-sha256 map from manifest_payload.files
    let listed_map: std::collections::HashMap<String, String> = {
        let files = manifest_payload.get("files");
        match files {
            None | Some(serde_json::Value::Null) => Default::default(),
            Some(serde_json::Value::Object(m)) => m
                .iter()
                .filter_map(|(k, v)| {
                    v.get("sha256")
                        .and_then(|s| s.as_str())
                        .map(|s| (k.clone(), s.to_string()))
                })
                .collect(),
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|item| {
                    let path = item.get("path")?.as_str()?.to_string();
                    let sha = item
                        .get("sha256")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some((path, sha))
                })
                .collect(),
            _ => return Err(unprocessable("manifest.files must be a map or list")),
        }
    };

    let mut normalized_files = Vec::new();
    for (path, data) in entries.iter() {
        let computed_sha = sha256_hex(data);
        if let Some(listed_sha) = listed_map.get(path) {
            if !listed_sha.is_empty() && listed_sha != &computed_sha {
                return Err(unprocessable(&format!(
                    "manifest hash mismatch for path: {path}"
                )));
            }
        }
        normalized_files.push(serde_json::json!({
            "path": path,
            "sha256": computed_sha,
            "size": data.len(),
        }));
    }

    let remove_paths: Vec<String> = match manifest_payload.get("remove_paths") {
        None | Some(serde_json::Value::Null) => vec![],
        Some(serde_json::Value::Array(arr)) => {
            let mut set = std::collections::BTreeSet::new();
            for v in arr {
                if let Some(s) = v.as_str() {
                    let normalized = s
                        .trim_start_matches('/')
                        .replace('\\', "/");
                    if !normalized.is_empty()
                        && !normalized.starts_with("../")
                        && !normalized.contains("/../")
                    {
                        set.insert(normalized);
                    }
                }
            }
            set.into_iter().collect()
        }
        _ => return Err(unprocessable("manifest.remove_paths must be a list")),
    };

    Ok(serde_json::json!({
        "format": "edgeplane-skill-bundle/v1",
        "scope_type": scope_type,
        "scope_id": scope_id,
        "domain_id": domain_id,
        "mission_id": mission_id,
        "files": normalized_files,
        "remove_paths": remove_paths,
    }))
}

// ---------------------------------------------------------------------------
// Row projection helpers
// ---------------------------------------------------------------------------

fn row_to_bundle(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let manifest_str: String = row.get("manifest_json");
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_str).unwrap_or(serde_json::json!({}));
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "scope_type": row.get::<String, _>("scope_type"),
        "scope_id": row.get::<String, _>("scope_id"),
        "domain_id": row.get::<String, _>("domain_id"),
        "mission_id": row.get::<String, _>("mission_id"),
        "version": row.get::<i32, _>("version"),
        "status": row.get::<String, _>("status"),
        "signature_alg": row.get::<String, _>("signature_alg"),
        "signing_key_id": row.get::<String, _>("signing_key_id"),
        "signature": row.get::<String, _>("signature"),
        "signature_verified": row.get::<bool, _>("signature_verified"),
        "manifest": manifest,
        "sha256": row.get::<String, _>("sha256"),
        "size_bytes": row.get::<i32, _>("size_bytes"),
        "created_by": row.get::<String, _>("created_by"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

// ---------------------------------------------------------------------------
// Request body types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct BundleCreate {
    tarball_b64: String,
    #[serde(default)]
    manifest: serde_json::Value,
    #[serde(default = "default_active")]
    status: String,
    #[serde(default)]
    signature_alg: String,
    #[serde(default)]
    signing_key_id: String,
    #[serde(default)]
    signature: String,
}

fn default_active() -> String {
    "active".to_string()
}

// ---------------------------------------------------------------------------
// Auth helpers
// ---------------------------------------------------------------------------

async fn can_write_domain(db: &sqlx::PgPool, principal: &Principal, domain_id: &str) -> bool {
    if principal.is_admin {
        return true;
    }
    if let Ok(Some(row)) =
        sqlx::query("SELECT owners, contributors FROM domain WHERE id=$1")
            .bind(domain_id)
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
        "SELECT EXISTS(SELECT 1 FROM domainrolemembership WHERE domain_id=$1 AND subject=$2 AND role IN ('owner','contributor'))",
    )
    .bind(domain_id)
    .bind(&principal.subject)
    .fetch_one(db)
    .await
    .unwrap_or(false)
}

/// Validate that a mission_id belongs to the given domain.
async fn validate_mission_scope(
    db: &sqlx::PgPool,
    domain_id: &str,
    mission_id: &str,
) -> Result<(), Response> {
    if mission_id.is_empty() {
        return Ok(());
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM mission WHERE id=$1 AND domain_id=$2)")
            .bind(mission_id)
            .bind(domain_id)
            .fetch_one(db)
            .await
            .unwrap_or(false);
    if !exists {
        return Err(not_found("Mission not found in domain"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Core bundle creation (shared between domain and mission scope)
// ---------------------------------------------------------------------------

async fn do_create_bundle(
    db: &sqlx::PgPool,
    principal: &Principal,
    scope_type: &str,
    scope_id: &str,
    domain_id: &str,
    mission_id: &str,
    body: &BundleCreate,
) -> Response {
    let status = body.status.as_str();
    if status != "active" && status != "deprecated" {
        return unprocessable("status must be active or deprecated");
    }
    let sig_alg = if body.signature_alg.trim().is_empty() {
        "hmac-sha256"
    } else {
        body.signature_alg.trim()
    };

    let tarball_bytes = match decode_tarball_b64(&body.tarball_b64) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let entries = match extract_tar_entries(&tarball_bytes) {
        Ok(e) => e,
        Err(r) => return r,
    };

    let manifest_payload = if body.manifest.is_null() {
        serde_json::json!({})
    } else {
        body.manifest.clone()
    };

    let manifest = match build_normalized_manifest(
        scope_type,
        scope_id,
        domain_id,
        mission_id,
        &manifest_payload,
        &entries,
    ) {
        Ok(m) => m,
        Err(r) => return r,
    };
    let tarball_sha256 = sha256_hex(&tarball_bytes);

    // Optional signature verification
    let mut signature_verified = false;
    let signing_secret = std::env::var("EP_SKILLS_SIGNING_SECRET").unwrap_or_default();
    let signing_secret = signing_secret.trim();
    if !signing_secret.is_empty() {
        let sig = body.signature.trim().to_lowercase();
        if sig.is_empty() {
            return unprocessable(
                "signature is required when signing verification is enabled",
            );
        }
        let payload_str = canon_json(&serde_json::json!({
            "manifest": &manifest,
            "signature_alg": sig_alg,
            "tarball_sha256": &tarball_sha256,
        }));
        let expected = {
            use hmac::{Hmac, Mac};
            type HmacSha256 = Hmac<sha2::Sha256>;
            let mut mac =
                HmacSha256::new_from_slice(signing_secret.as_bytes()).unwrap();
            mac.update(payload_str.as_bytes());
            hex::encode(mac.finalize().into_bytes())
        };
        if !constant_time_eq(&expected, &sig) {
            return unprocessable("Skill bundle signature verification failed");
        }
        signature_verified = true;
    }

    // Get next version number
    let latest_version: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) FROM skillbundle WHERE scope_type=$1 AND scope_id=$2",
    )
    .bind(scope_type)
    .bind(scope_id)
    .fetch_one(db)
    .await
    .unwrap_or(0);
    let next_version = latest_version + 1;

    let tarball_stored =
        base64::engine::general_purpose::STANDARD.encode(&tarball_bytes);
    let manifest_json = canon_json(&manifest);
    let now = Utc::now().naive_utc();

    // Generate unique ID
    let mut bundle_id = new_hash_id();
    loop {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM skillbundle WHERE id=$1)")
                .bind(&bundle_id)
                .fetch_one(db)
                .await
                .unwrap_or(false);
        if !exists {
            break;
        }
        bundle_id = new_hash_id();
    }

    let result = sqlx::query(
        "INSERT INTO skillbundle \
         (id, scope_type, scope_id, domain_id, mission_id, version, status, \
          signature_alg, signing_key_id, signature, signature_verified, \
          manifest_json, tarball_b64, sha256, size_bytes, created_by, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$17) RETURNING *",
    )
    .bind(&bundle_id)
    .bind(scope_type)
    .bind(scope_id)
    .bind(domain_id)
    .bind(mission_id)
    .bind(next_version)
    .bind(status)
    .bind(sig_alg)
    .bind(&body.signing_key_id)
    .bind(body.signature.trim().to_lowercase())
    .bind(signature_verified)
    .bind(&manifest_json)
    .bind(&tarball_stored)
    .bind(&tarball_sha256)
    .bind(tarball_bytes.len() as i32)
    .bind(&principal.subject)
    .bind(now)
    .fetch_one(db)
    .await;

    match result {
        Ok(row) => (StatusCode::CREATED, Json(row_to_bundle(&row))).into_response(),
        Err(e) => {
            tracing::error!("create_bundle insert: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn create_domain_bundle(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
    Json(body): Json<BundleCreate>,
) -> impl IntoResponse {
    if !can_write_domain(&state.db, &principal, &domain_id).await {
        return forbidden();
    }
    do_create_bundle(
        &state.db,
        &principal,
        "domain",
        &domain_id.clone(),
        &domain_id,
        "",
        &body,
    )
    .await
}

async fn create_mission_bundle(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((domain_id, mission_id)): Path<(String, String)>,
    Json(body): Json<BundleCreate>,
) -> impl IntoResponse {
    if !can_write_domain(&state.db, &principal, &domain_id).await {
        return forbidden();
    }
    if let Err(r) = validate_mission_scope(&state.db, &domain_id, &mission_id).await {
        return r;
    }
    do_create_bundle(
        &state.db,
        &principal,
        "mission",
        &mission_id.clone(),
        &domain_id,
        &mission_id,
        &body,
    )
    .await
}

async fn deprecate_bundle(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((domain_id, bundle_id)): Path<(String, String)>,
) -> impl IntoResponse {
    // Owners/admin only for deprecation
    if !principal.is_admin {
        let owned = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM domain WHERE id=$1 AND (owners ILIKE $2 OR owners ILIKE $3 OR owners ILIKE $4))",
        )
        .bind(&domain_id)
        .bind(format!("%{}%", principal.subject.to_lowercase()))
        .bind(format!("{},%", principal.subject.to_lowercase()))
        .bind(principal.subject.to_lowercase())
        .fetch_one(&state.db)
        .await
        .unwrap_or(false);

        if !owned {
            // Also check domainrolemembership for owner role
            let role_owned = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM domainrolemembership WHERE domain_id=$1 AND subject=$2 AND role='owner')",
            )
            .bind(&domain_id)
            .bind(&principal.subject)
            .fetch_one(&state.db)
            .await
            .unwrap_or(false);

            if !role_owned {
                return forbidden();
            }
        }
    }

    let now = Utc::now().naive_utc();
    let result = sqlx::query(
        "UPDATE skillbundle SET status='deprecated', updated_at=$3 \
         WHERE id=$1 AND domain_id=$2 RETURNING *",
    )
    .bind(&bundle_id)
    .bind(&domain_id)
    .bind(now)
    .fetch_optional(&state.db)
    .await;

    match result {
        Ok(Some(row)) => Json(row_to_bundle(&row)).into_response(),
        Ok(None) => not_found("Skill bundle not found"),
        Err(e) => {
            tracing::error!("deprecate_bundle update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
