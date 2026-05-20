use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
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
            "/missions/{mission_id}/skills/bundles",
            post(create_mission_bundle),
        )
        .route(
            "/missions/{mission_id}/klusters/{kluster_id}/skills/bundles",
            post(create_kluster_bundle),
        )
        .route("/skills/snapshots/resolve", get(resolve_snapshot))
        .route(
            "/skills/snapshots/{snapshot_id}/download",
            get(download_snapshot),
        )
        .route("/skills/sync/status", get(sync_status))
        .route("/skills/sync/ack", post(sync_ack))
        .route(
            "/missions/{mission_id}/skills/bundles/{bundle_id}/deprecate",
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

fn encode_tar_entries(entries: &std::collections::BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use tar::{Builder, Header};

    let buf = Vec::new();
    let gz = GzEncoder::new(buf, Compression::default());
    let mut archive = Builder::new(gz);

    for (path, data) in entries.iter() {
        let mut header = Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        let _ = archive.append_data(&mut header, path, data.as_slice());
    }

    archive
        .into_inner()
        .and_then(|gz| gz.finish())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Manifest builder
// ---------------------------------------------------------------------------

fn build_normalized_manifest(
    scope_type: &str,
    scope_id: &str,
    mission_id: &str,
    kluster_id: &str,
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
        "format": "mc-skill-bundle/v1",
        "scope_type": scope_type,
        "scope_id": scope_id,
        "mission_id": mission_id,
        "kluster_id": kluster_id,
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
        "mission_id": row.get::<String, _>("mission_id"),
        "kluster_id": row.get::<String, _>("kluster_id"),
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

fn row_to_snapshot(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let manifest_str: String = row.get("manifest_json");
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_str).unwrap_or(serde_json::json!({}));
    serde_json::json!({
        "snapshot_id": row.get::<String, _>("id"),
        "mission_id": row.get::<String, _>("mission_id"),
        "kluster_id": row.get::<String, _>("kluster_id"),
        "mission_bundle_id": row.get::<String, _>("mission_bundle_id"),
        "kluster_bundle_id": row.get::<String, _>("kluster_bundle_id"),
        "effective_version": row.get::<String, _>("effective_version"),
        "sha256": row.get::<String, _>("sha256"),
        "size_bytes": row.get::<i32, _>("size_bytes"),
        "manifest": manifest,
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
    })
}

fn row_to_snapshot_download(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let manifest_str: String = row.get("manifest_json");
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_str).unwrap_or(serde_json::json!({}));
    serde_json::json!({
        "snapshot_id": row.get::<String, _>("id"),
        "sha256": row.get::<String, _>("sha256"),
        "tarball_b64": row.get::<String, _>("tarball_b64"),
        "size_bytes": row.get::<i32, _>("size_bytes"),
        "manifest": manifest,
    })
}

fn row_to_localstate(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let drift_str: String = row.get("drift_details_json");
    let drift: serde_json::Value =
        serde_json::from_str(&drift_str).unwrap_or(serde_json::json!({}));
    serde_json::json!({
        "mission_id": row.get::<String, _>("mission_id"),
        "kluster_id": row.get::<String, _>("kluster_id"),
        "actor_subject": row.get::<String, _>("actor_subject"),
        "agent_id": row.get::<String, _>("agent_id"),
        "last_snapshot_id": row.get::<String, _>("last_snapshot_id"),
        "last_snapshot_sha256": row.get::<String, _>("last_snapshot_sha256"),
        "local_overlay_sha256": row.get::<String, _>("local_overlay_sha256"),
        "degraded_offline": row.get::<bool, _>("degraded_offline"),
        "drift_flag": row.get::<bool, _>("drift_flag"),
        "drift_details": drift,
        "last_sync_at": row.get::<Option<chrono::NaiveDateTime>, _>("last_sync_at"),
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

#[derive(Deserialize)]
struct ResolveQuery {
    mission_id: String,
    #[serde(default)]
    kluster_id: String,
}

#[derive(Deserialize)]
struct SyncStatusQuery {
    mission_id: String,
    #[serde(default)]
    kluster_id: String,
    #[serde(default)]
    agent_id: String,
}

#[derive(Deserialize)]
struct SyncAckBody {
    mission_id: String,
    #[serde(default)]
    kluster_id: String,
    #[serde(default)]
    agent_id: String,
    snapshot_id: String,
    snapshot_sha256: String,
    #[serde(default)]
    local_overlay_sha256: String,
    #[serde(default)]
    degraded_offline: bool,
    #[serde(default)]
    drift_flag: bool,
    #[serde(default)]
    drift_details: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Auth helpers
// ---------------------------------------------------------------------------

async fn can_read_mission(db: &sqlx::PgPool, principal: &Principal, mission_id: &str) -> bool {
    if principal.is_admin {
        return true;
    }
    if let Ok(Some(row)) =
        sqlx::query("SELECT visibility, owners, contributors FROM mission WHERE id=$1")
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

/// Validate that a kluster_id belongs to the given mission.
async fn validate_kluster_scope(
    db: &sqlx::PgPool,
    mission_id: &str,
    kluster_id: &str,
) -> Result<(), Response> {
    if kluster_id.is_empty() {
        return Ok(());
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM kluster WHERE id=$1 AND mission_id=$2)")
            .bind(kluster_id)
            .bind(mission_id)
            .fetch_one(db)
            .await
            .unwrap_or(false);
    if !exists {
        return Err(not_found("Kluster not found in mission"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Core bundle creation (shared between mission and kluster scope)
// ---------------------------------------------------------------------------

async fn do_create_bundle(
    db: &sqlx::PgPool,
    principal: &Principal,
    scope_type: &str,
    scope_id: &str,
    mission_id: &str,
    kluster_id: &str,
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
        mission_id,
        kluster_id,
        &manifest_payload,
        &entries,
    ) {
        Ok(m) => m,
        Err(r) => return r,
    };
    let tarball_sha256 = sha256_hex(&tarball_bytes);

    // Optional signature verification
    let mut signature_verified = false;
    let signing_secret = std::env::var("MC_SKILLS_SIGNING_SECRET").unwrap_or_default();
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
         (id, scope_type, scope_id, mission_id, kluster_id, version, status, \
          signature_alg, signing_key_id, signature, signature_verified, \
          manifest_json, tarball_b64, sha256, size_bytes, created_by, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$17) RETURNING *",
    )
    .bind(&bundle_id)
    .bind(scope_type)
    .bind(scope_id)
    .bind(mission_id)
    .bind(kluster_id)
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

async fn create_mission_bundle(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
    Json(body): Json<BundleCreate>,
) -> impl IntoResponse {
    if !can_write_mission(&state.db, &principal, &mission_id).await {
        return forbidden();
    }
    do_create_bundle(
        &state.db,
        &principal,
        "mission",
        &mission_id.clone(),
        &mission_id,
        "",
        &body,
    )
    .await
}

async fn create_kluster_bundle(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((mission_id, kluster_id)): Path<(String, String)>,
    Json(body): Json<BundleCreate>,
) -> impl IntoResponse {
    if !can_write_mission(&state.db, &principal, &mission_id).await {
        return forbidden();
    }
    if let Err(r) = validate_kluster_scope(&state.db, &mission_id, &kluster_id).await {
        return r;
    }
    do_create_bundle(
        &state.db,
        &principal,
        "kluster",
        &kluster_id.clone(),
        &mission_id,
        &kluster_id,
        &body,
    )
    .await
}

async fn resolve_snapshot(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<ResolveQuery>,
) -> impl IntoResponse {
    if !can_read_mission(&state.db, &principal, &q.mission_id).await {
        return forbidden();
    }
    if !q.kluster_id.is_empty() {
        if let Err(r) =
            validate_kluster_scope(&state.db, &q.mission_id, &q.kluster_id).await
        {
            return r;
        }
    }

    // Load active mission bundle (latest by version)
    let mission_bundle_row = match sqlx::query(
        "SELECT * FROM skillbundle \
         WHERE scope_type='mission' AND scope_id=$1 AND status='active' \
         ORDER BY version DESC LIMIT 1",
    )
    .bind(&q.mission_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("No active mission skill bundle found"),
        Err(e) => {
            tracing::error!("resolve_snapshot fetch mission bundle: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mission_bundle_id: String = mission_bundle_row.get("id");
    let mission_bundle_sha: String = mission_bundle_row.get("sha256");
    let mission_bundle_version: i32 = mission_bundle_row.get("version");
    let mission_tarball_b64: String = mission_bundle_row.get("tarball_b64");
    let mission_manifest_str: String = mission_bundle_row.get("manifest_json");
    let mission_manifest: serde_json::Value =
        serde_json::from_str(&mission_manifest_str).unwrap_or(serde_json::json!({}));

    // Optionally load kluster bundle
    let kluster_bundle_data: Option<(String, String, i32, String, serde_json::Value)> =
        if !q.kluster_id.is_empty() {
            match sqlx::query(
                "SELECT * FROM skillbundle \
                 WHERE scope_type='kluster' AND scope_id=$1 AND status='active' \
                 ORDER BY version DESC LIMIT 1",
            )
            .bind(&q.kluster_id)
            .fetch_optional(&state.db)
            .await
            {
                Ok(Some(r)) => {
                    let kid: String = r.get("id");
                    let ksha: String = r.get("sha256");
                    let kver: i32 = r.get("version");
                    let ktarball: String = r.get("tarball_b64");
                    let kmanifest_str: String = r.get("manifest_json");
                    let kmanifest: serde_json::Value =
                        serde_json::from_str(&kmanifest_str)
                            .unwrap_or(serde_json::json!({}));
                    Some((kid, ksha, kver, ktarball, kmanifest))
                }
                Ok(None) => None,
                Err(e) => {
                    tracing::error!("resolve_snapshot fetch kluster bundle: {e}");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
        } else {
            None
        };

    // Decode mission tarball
    let mission_bytes = match decode_tarball_b64(&mission_tarball_b64) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let mission_entries = match extract_tar_entries(&mission_bytes) {
        Ok(e) => e,
        Err(r) => return r,
    };

    // Build manifest files map from mission
    let mut effective_manifest_files: std::collections::BTreeMap<
        String,
        serde_json::Value,
    > = Default::default();
    if let Some(serde_json::Value::Array(files)) = mission_manifest.get("files") {
        for item in files {
            if let Some(path) = item.get("path").and_then(|v| v.as_str()) {
                effective_manifest_files.insert(path.to_string(), item.clone());
            }
        }
    }

    let mut effective_entries: std::collections::BTreeMap<String, Vec<u8>> =
        mission_entries.clone();

    let (kluster_bundle_id, kluster_bundle_sha, kluster_bundle_version) =
        if let Some((kid, ksha, kver, ktarball, kmanifest)) = kluster_bundle_data {
            // Decode kluster tarball
            let kluster_bytes = match decode_tarball_b64(&ktarball) {
                Ok(b) => b,
                Err(r) => return r,
            };
            let kluster_entries = match extract_tar_entries(&kluster_bytes) {
                Ok(e) => e,
                Err(r) => return r,
            };

            // Apply remove_paths from kluster manifest
            if let Some(serde_json::Value::Array(removes)) =
                kmanifest.get("remove_paths")
            {
                for rp in removes {
                    if let Some(path) = rp.as_str() {
                        effective_entries.remove(path);
                        effective_manifest_files.remove(path);
                    }
                }
            }

            // Overlay kluster entries
            for (path, data) in kluster_entries.iter() {
                effective_entries.insert(path.clone(), data.clone());
            }

            // Overlay kluster manifest files
            if let Some(serde_json::Value::Array(files)) = kmanifest.get("files") {
                for item in files {
                    if let Some(path) = item.get("path").and_then(|v| v.as_str()) {
                        effective_manifest_files
                            .insert(path.to_string(), item.clone());
                    }
                }
            }

            (kid, ksha, kver)
        } else {
            (String::new(), String::new(), 0i32)
        };

    // Build effective manifest
    let effective_manifest = serde_json::json!({
        "format": "mc-skill-snapshot/v1",
        "mission_id": &q.mission_id,
        "kluster_id": &q.kluster_id,
        "source": {
            "mission_bundle_id": &mission_bundle_id,
            "kluster_bundle_id": &kluster_bundle_id,
            "mission_bundle_version": mission_bundle_version,
            "kluster_bundle_version": kluster_bundle_version,
        },
        "files": effective_manifest_files.values().collect::<Vec<_>>(),
    });

    // Compute signature hash for dedup
    let signature_payload = serde_json::json!({
        "mission_bundle_sha": &mission_bundle_sha,
        "kluster_bundle_sha": &kluster_bundle_sha,
        "manifest": &effective_manifest,
    });
    let snapshot_sha = sha256_hex(canon_json(&signature_payload).as_bytes());

    // Check for existing snapshot with same sha256
    let existing = sqlx::query(
        "SELECT * FROM skillsnapshot \
         WHERE mission_id=$1 AND kluster_id=$2 AND sha256=$3 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&q.mission_id)
    .bind(&q.kluster_id)
    .bind(&snapshot_sha)
    .fetch_optional(&state.db)
    .await;

    match existing {
        Ok(Some(row)) => return Json(row_to_snapshot(&row)).into_response(),
        Ok(None) => {} // fall through to create
        Err(e) => {
            tracing::error!("resolve_snapshot check existing: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    // Encode merged entries to new tarball
    let tar_bytes = encode_tar_entries(&effective_entries);
    let effective_version = format!(
        "m{}-k{}",
        mission_bundle_version, kluster_bundle_version
    );
    let tarball_stored = base64::engine::general_purpose::STANDARD.encode(&tar_bytes);
    let manifest_json = canon_json(&effective_manifest);
    let now = Utc::now().naive_utc();

    let mut snapshot_id = new_hash_id();
    loop {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM skillsnapshot WHERE id=$1)")
                .bind(&snapshot_id)
                .fetch_one(&state.db)
                .await
                .unwrap_or(false);
        if !exists {
            break;
        }
        snapshot_id = new_hash_id();
    }

    let result = sqlx::query(
        "INSERT INTO skillsnapshot \
         (id, mission_id, kluster_id, mission_bundle_id, kluster_bundle_id, \
          effective_version, manifest_json, tarball_b64, sha256, size_bytes, \
          created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$11) RETURNING *",
    )
    .bind(&snapshot_id)
    .bind(&q.mission_id)
    .bind(&q.kluster_id)
    .bind(&mission_bundle_id)
    .bind(&kluster_bundle_id)
    .bind(&effective_version)
    .bind(&manifest_json)
    .bind(&tarball_stored)
    .bind(&snapshot_sha)
    .bind(tar_bytes.len() as i32)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => Json(row_to_snapshot(&row)).into_response(),
        Err(e) => {
            tracing::error!("resolve_snapshot insert: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn download_snapshot(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(snapshot_id): Path<String>,
) -> impl IntoResponse {
    let row = match sqlx::query("SELECT * FROM skillsnapshot WHERE id=$1")
        .bind(&snapshot_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Skill snapshot not found"),
        Err(e) => {
            tracing::error!("download_snapshot fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mission_id: String = row.get("mission_id");
    if !can_read_mission(&state.db, &principal, &mission_id).await {
        return forbidden();
    }

    Json(row_to_snapshot_download(&row)).into_response()
}

async fn sync_status(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<SyncStatusQuery>,
) -> impl IntoResponse {
    if !can_read_mission(&state.db, &principal, &q.mission_id).await {
        return forbidden();
    }
    if !q.kluster_id.is_empty() {
        if let Err(r) =
            validate_kluster_scope(&state.db, &q.mission_id, &q.kluster_id).await
        {
            return r;
        }
    }

    let row = sqlx::query(
        "SELECT * FROM skilllocalstate \
         WHERE actor_subject=$1 AND mission_id=$2 AND kluster_id=$3 AND agent_id=$4 \
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(&principal.subject)
    .bind(&q.mission_id)
    .bind(&q.kluster_id)
    .bind(&q.agent_id)
    .fetch_optional(&state.db)
    .await;

    match row {
        Ok(Some(r)) => Json(row_to_localstate(&r)).into_response(),
        Ok(None) => {
            // Return zero-state
            Json(serde_json::json!({
                "mission_id": &q.mission_id,
                "kluster_id": &q.kluster_id,
                "actor_subject": &principal.subject,
                "agent_id": &q.agent_id,
                "last_snapshot_id": "",
                "last_snapshot_sha256": "",
                "local_overlay_sha256": "",
                "degraded_offline": false,
                "drift_flag": false,
                "drift_details": {},
                "last_sync_at": null,
                "updated_at": null,
            }))
            .into_response()
        }
        Err(e) => {
            tracing::error!("sync_status query: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn sync_ack(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<SyncAckBody>,
) -> impl IntoResponse {
    if !can_read_mission(&state.db, &principal, &body.mission_id).await {
        return forbidden();
    }
    if !body.kluster_id.is_empty() {
        if let Err(r) =
            validate_kluster_scope(&state.db, &body.mission_id, &body.kluster_id).await
        {
            return r;
        }
    }

    let drift_json = if body.drift_details.is_null() || body.drift_details.as_object().map_or(false, |m| m.is_empty()) {
        "{}".to_string()
    } else {
        canon_json(&body.drift_details)
    };

    let now = Utc::now().naive_utc();

    // Try UPSERT via INSERT ON CONFLICT
    let result = sqlx::query(
        "INSERT INTO skilllocalstate \
         (actor_subject, mission_id, kluster_id, agent_id, \
          last_snapshot_id, last_snapshot_sha256, local_overlay_sha256, \
          degraded_offline, drift_flag, drift_details_json, \
          last_sync_at, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$11,$11) \
         ON CONFLICT (actor_subject, mission_id, kluster_id, agent_id) DO UPDATE SET \
          last_snapshot_id = EXCLUDED.last_snapshot_id, \
          last_snapshot_sha256 = EXCLUDED.last_snapshot_sha256, \
          local_overlay_sha256 = EXCLUDED.local_overlay_sha256, \
          degraded_offline = EXCLUDED.degraded_offline, \
          drift_flag = EXCLUDED.drift_flag, \
          drift_details_json = EXCLUDED.drift_details_json, \
          last_sync_at = EXCLUDED.last_sync_at, \
          updated_at = EXCLUDED.updated_at \
         RETURNING *",
    )
    .bind(&principal.subject)
    .bind(&body.mission_id)
    .bind(&body.kluster_id)
    .bind(&body.agent_id)
    .bind(&body.snapshot_id)
    .bind(&body.snapshot_sha256)
    .bind(&body.local_overlay_sha256)
    .bind(body.degraded_offline)
    .bind(body.drift_flag)
    .bind(&drift_json)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => Json(row_to_localstate(&row)).into_response(),
        Err(e) => {
            // ON CONFLICT might fail if no unique constraint; fall back to SELECT + UPDATE/INSERT
            tracing::warn!("sync_ack upsert failed ({e}), falling back to manual upsert");

            let existing = sqlx::query(
                "SELECT * FROM skilllocalstate \
                 WHERE actor_subject=$1 AND mission_id=$2 AND kluster_id=$3 AND agent_id=$4 \
                 LIMIT 1",
            )
            .bind(&principal.subject)
            .bind(&body.mission_id)
            .bind(&body.kluster_id)
            .bind(&body.agent_id)
            .fetch_optional(&state.db)
            .await;

            match existing {
                Ok(Some(existing_row)) => {
                    let id: i32 = existing_row.get("id");
                    match sqlx::query(
                        "UPDATE skilllocalstate SET \
                         last_snapshot_id=$2, last_snapshot_sha256=$3, \
                         local_overlay_sha256=$4, degraded_offline=$5, \
                         drift_flag=$6, drift_details_json=$7, \
                         last_sync_at=$8, updated_at=$8 \
                         WHERE id=$1 RETURNING *",
                    )
                    .bind(id)
                    .bind(&body.snapshot_id)
                    .bind(&body.snapshot_sha256)
                    .bind(&body.local_overlay_sha256)
                    .bind(body.degraded_offline)
                    .bind(body.drift_flag)
                    .bind(&drift_json)
                    .bind(now)
                    .fetch_one(&state.db)
                    .await
                    {
                        Ok(row) => Json(row_to_localstate(&row)).into_response(),
                        Err(e2) => {
                            tracing::error!("sync_ack update fallback: {e2}");
                            StatusCode::INTERNAL_SERVER_ERROR.into_response()
                        }
                    }
                }
                Ok(None) => {
                    match sqlx::query(
                        "INSERT INTO skilllocalstate \
                         (actor_subject, mission_id, kluster_id, agent_id, \
                          last_snapshot_id, last_snapshot_sha256, local_overlay_sha256, \
                          degraded_offline, drift_flag, drift_details_json, \
                          last_sync_at, created_at, updated_at) \
                         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$11,$11) RETURNING *",
                    )
                    .bind(&principal.subject)
                    .bind(&body.mission_id)
                    .bind(&body.kluster_id)
                    .bind(&body.agent_id)
                    .bind(&body.snapshot_id)
                    .bind(&body.snapshot_sha256)
                    .bind(&body.local_overlay_sha256)
                    .bind(body.degraded_offline)
                    .bind(body.drift_flag)
                    .bind(&drift_json)
                    .bind(now)
                    .fetch_one(&state.db)
                    .await
                    {
                        Ok(row) => Json(row_to_localstate(&row)).into_response(),
                        Err(e2) => {
                            tracing::error!("sync_ack insert fallback: {e2}");
                            StatusCode::INTERNAL_SERVER_ERROR.into_response()
                        }
                    }
                }
                Err(e2) => {
                    tracing::error!("sync_ack select fallback: {e2}");
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                }
            }
        }
    }
}

async fn deprecate_bundle(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((mission_id, bundle_id)): Path<(String, String)>,
) -> impl IntoResponse {
    // Owners/admin only for deprecation
    if !principal.is_admin {
        let owned = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM mission WHERE id=$1 AND (owners ILIKE $2 OR owners ILIKE $3 OR owners ILIKE $4))",
        )
        .bind(&mission_id)
        .bind(format!("%{}%", principal.subject.to_lowercase()))
        .bind(format!("{},%", principal.subject.to_lowercase()))
        .bind(principal.subject.to_lowercase())
        .fetch_one(&state.db)
        .await
        .unwrap_or(false);

        if !owned {
            // Also check missionrolemembership for owner role
            let role_owned = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM missionrolemembership WHERE mission_id=$1 AND subject=$2 AND role='owner')",
            )
            .bind(&mission_id)
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
         WHERE id=$1 AND mission_id=$2 RETURNING *",
    )
    .bind(&bundle_id)
    .bind(&mission_id)
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
