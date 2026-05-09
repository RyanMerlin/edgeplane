use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::Utc;
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::Arc;
use tar::Builder;
use uuid::Uuid;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/packs", get(list_packs))
        .route("/packs/{pack_id}", get(get_pack).delete(delete_pack))
        .route("/packs/missions/{mission_id}/export", post(export_mission))
        .route("/packs/{pack_id}/install", post(install_pack))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn split_csv(s: &str) -> Vec<String> {
    s.split(',').map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect()
}

fn new_hash_id() -> String {
    let bytes: [u8; 6] = rand::random();
    hex::encode(bytes)
}

fn add_json_to_tar<W: std::io::Write>(
    builder: &mut Builder<W>,
    path: &str,
    value: &serde_json::Value,
) -> std::io::Result<()> {
    let content = serde_json::to_vec_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_cksum();
    builder.append_data(&mut header, path, Cursor::new(content))
}

// ── List / Get / Delete ───────────────────────────────────────────────────────

async fn list_packs(
    State(state): State<Arc<AppState>>,
    principal: Principal,
) -> impl IntoResponse {
    match sqlx::query(
        "SELECT id, name, version, sha256, created_at \
         FROM missionpack \
         WHERE owner_subject=$1 \
         ORDER BY created_at DESC",
    )
    .bind(&principal.subject)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => {
            let items: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| {
                    let sha256_full: String = row.get("sha256");
                    let sha256_short = sha256_full.chars().take(8).collect::<String>();
                    serde_json::json!({
                        "id": row.get::<String, _>("id"),
                        "name": row.get::<String, _>("name"),
                        "version": row.get::<i32, _>("version"),
                        "sha256": sha256_short,
                        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
                    })
                })
                .collect();
            Json(items).into_response()
        }
        Err(e) => {
            tracing::error!("list_packs: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_pack(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(pack_id): Path<String>,
) -> impl IntoResponse {
    match sqlx::query(
        "SELECT id, name, version, sha256, manifest_json \
         FROM missionpack \
         WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&pack_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => {
            let manifest_str: String = row.get("manifest_json");
            let manifest: serde_json::Value =
                serde_json::from_str(&manifest_str).unwrap_or(serde_json::json!({}));
            Json(serde_json::json!({
                "id": row.get::<String, _>("id"),
                "name": row.get::<String, _>("name"),
                "version": row.get::<i32, _>("version"),
                "sha256": row.get::<String, _>("sha256"),
                "manifest": manifest,
            }))
            .into_response()
        }
        Ok(None) => not_found("Pack not found"),
        Err(e) => {
            tracing::error!("get_pack: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn delete_pack(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(pack_id): Path<String>,
) -> impl IntoResponse {
    match sqlx::query("DELETE FROM missionpack WHERE id=$1 AND owner_subject=$2")
        .bind(&pack_id)
        .bind(&principal.subject)
        .execute(&state.db)
        .await
    {
        Ok(r) if r.rows_affected() == 0 => not_found("Pack not found"),
        Ok(_) => Json(serde_json::json!({"deleted": pack_id})).into_response(),
        Err(e) => {
            tracing::error!("delete_pack: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ── Export ────────────────────────────────────────────────────────────────────

async fn export_mission(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
) -> impl IntoResponse {
    // 1. Load mission + verify access
    let mission_row = sqlx::query("SELECT * FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await;
    let mission_row = match mission_row {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Mission not found"),
        Err(e) => { tracing::error!("export_mission fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    let owners: String = mission_row.try_get("owners").unwrap_or_default();
    let subject_lower = principal.subject.to_lowercase();
    if !principal.is_admin && !split_csv(&owners).contains(&subject_lower) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let mission_name: String = mission_row.get("name");
    let mission_desc: String = mission_row.try_get("description").unwrap_or_default();

    // 2. Load klusters
    let klusters = sqlx::query(
        "SELECT id, name, description FROM kluster WHERE mission_id=$1 ORDER BY created_at ASC",
    )
    .bind(&mission_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // 3. Load active skill bundles scoped to this mission
    let skills = sqlx::query(
        "SELECT id, version, tarball_b64, sha256, manifest_json \
         FROM skillbundle WHERE scope_type='mission' AND scope_id=$1 AND status='active'",
    )
    .bind(&mission_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // 4. Load budget policies for this mission owned by the caller
    let budgets = sqlx::query(
        "SELECT scope_type, window_type, hard_cap_cents, soft_cap_cents, action_on_breach \
         FROM budgetpolicy WHERE scope_type='mission' AND scope_id=$1 AND owner_subject=$2 AND active=true",
    )
    .bind(&mission_id)
    .bind(&principal.subject)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // 5. Build tar.gz in memory
    let manifest = serde_json::json!({
        "version": 1,
        "mission_id": mission_id,
        "mission_name": mission_name,
        "kluster_count": klusters.len(),
        "skill_count": skills.len(),
        "budget_count": budgets.len(),
        "exported_at": Utc::now().naive_utc().format("%Y-%m-%dT%H:%M:%S").to_string(),
    });

    let tarball_bytes: Vec<u8> = {
        let buf = Vec::new();
        let enc = GzEncoder::new(buf, Compression::default());
        let mut builder = Builder::new(enc);

        let _ = add_json_to_tar(&mut builder, "mission.json", &serde_json::json!({
            "id": mission_id,
            "name": mission_name,
            "description": mission_desc,
        }));

        for k in &klusters {
            let k_id: String = k.get("id");
            let k_name: String = k.get("name");
            let k_desc: String = k.try_get("description").unwrap_or_default();
            let _ = add_json_to_tar(&mut builder, &format!("klusters/{k_id}.json"), &serde_json::json!({
                "id": k_id,
                "name": k_name,
                "description": k_desc,
            }));
        }

        for s in &skills {
            let s_id: String = s.get("id");
            let s_version: i32 = s.try_get("version").unwrap_or(1);
            let s_tarball: String = s.try_get("tarball_b64").unwrap_or_default();
            let s_sha256: String = s.try_get("sha256").unwrap_or_default();
            let _ = add_json_to_tar(&mut builder, &format!("skills/{s_id}.json"), &serde_json::json!({
                "id": s_id,
                "version": s_version,
                "tarball_b64": s_tarball,
                "sha256": s_sha256,
            }));
        }

        for (idx, b) in budgets.iter().enumerate() {
            let _ = add_json_to_tar(&mut builder, &format!("budgets/{idx}.json"), &serde_json::json!({
                "scope_type": b.try_get::<String, _>("scope_type").unwrap_or_default(),
                "window_type": b.try_get::<String, _>("window_type").unwrap_or_default(),
                "hard_cap_cents": b.try_get::<i32, _>("hard_cap_cents").unwrap_or(0),
                "soft_cap_cents": b.try_get::<Option<i32>, _>("soft_cap_cents").unwrap_or(None),
                "action_on_breach": b.try_get::<String, _>("action_on_breach").unwrap_or_default(),
            }));
        }

        let _ = add_json_to_tar(&mut builder, "manifest.json", &manifest);

        match builder.into_inner() {
            Ok(gz) => match gz.finish() {
                Ok(bytes) => bytes,
                Err(e) => { tracing::error!("export_mission gz finish: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
            },
            Err(e) => { tracing::error!("export_mission tar into_inner: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
        }
    };

    // 6. SHA256 + base64
    let sha256 = {
        let mut hasher = Sha256::new();
        hasher.update(&tarball_bytes);
        format!("{:x}", hasher.finalize())
    };
    let tarball_b64 = STANDARD.encode(&tarball_bytes);
    let manifest_json = serde_json::to_string(&manifest).unwrap_or_else(|_| "{}".into());

    // 7. Resolve version (bump if same name already exists for this owner)
    let next_version: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) + 1 FROM missionpack WHERE owner_subject=$1 AND name=$2",
    )
    .bind(&principal.subject)
    .bind(&mission_name)
    .fetch_one(&state.db)
    .await
    .unwrap_or(1);

    let pack_id = Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();

    match sqlx::query(
        "INSERT INTO missionpack (id, owner_subject, name, version, sha256, tarball_b64, manifest_json, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$8) RETURNING id, name, sha256, version",
    )
    .bind(&pack_id)
    .bind(&principal.subject)
    .bind(&mission_name)
    .bind(next_version)
    .bind(&sha256)
    .bind(&tarball_b64)
    .bind(&manifest_json)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => Json(serde_json::json!({
            "pack_id": row.get::<String, _>("id"),
            "name": row.get::<String, _>("name"),
            "version": row.get::<i32, _>("version"),
            "sha256": row.get::<String, _>("sha256"),
        })).into_response(),
        Err(e) => { tracing::error!("export_mission insert: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

// ── Install ───────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct InstallQuery {
    target_mission_id: Option<String>,
}

async fn install_pack(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(pack_id): Path<String>,
    Query(q): Query<InstallQuery>,
) -> impl IntoResponse {
    // 1. Load pack
    let pack_row = sqlx::query(
        "SELECT tarball_b64 FROM missionpack WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&pack_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await;
    let pack_row = match pack_row {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Pack not found"),
        Err(e) => { tracing::error!("install_pack fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    // 2. Decode tarball
    let tarball_b64: String = pack_row.get("tarball_b64");
    let tarball_bytes = match STANDARD.decode(&tarball_b64) {
        Ok(b) => b,
        Err(e) => { tracing::error!("install_pack decode: {e}"); return (StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": "invalid tarball encoding"}))).into_response(); }
    };

    // 3. Extract all tar entries into memory
    let mut entries: HashMap<String, Vec<u8>> = HashMap::new();
    {
        let dec = GzDecoder::new(Cursor::new(&tarball_bytes));
        let mut archive = tar::Archive::new(dec);
        match archive.entries() {
            Ok(iter) => {
                for entry in iter {
                    let mut e = match entry { Ok(e) => e, Err(_) => continue };
                    let path = e.path().ok().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                    let mut content = Vec::new();
                    if e.read_to_end(&mut content).is_ok() {
                        entries.insert(path, content);
                    }
                }
            }
            Err(e) => { tracing::error!("install_pack tar entries: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
        }
    }

    // 4. Parse mission.json
    let mission_spec: serde_json::Value = entries.get("mission.json")
        .and_then(|b| serde_json::from_slice(b).ok())
        .unwrap_or(serde_json::json!({}));
    let mission_name = mission_spec.get("name").and_then(|v| v.as_str()).unwrap_or("Imported Mission");
    let mission_desc = mission_spec.get("description").and_then(|v| v.as_str()).unwrap_or("");

    let now = Utc::now().naive_utc();
    let mut created_missions: Vec<String> = vec![];
    let mut created_klusters: Vec<String> = vec![];
    let mut created_skills: Vec<String> = vec![];
    let mut created_budgets: Vec<String> = vec![];

    // 5. Find or create target mission
    let mission_id = if let Some(ref mid) = q.target_mission_id {
        let exists = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM mission WHERE id=$1)")
            .bind(mid)
            .fetch_one(&state.db)
            .await
            .unwrap_or(false);
        if !exists {
            return not_found("Target mission not found");
        }
        mid.clone()
    } else {
        let new_id = new_hash_id();
        let new_name = format!("{} (from pack)", mission_name);
        match sqlx::query(
            "INSERT INTO mission \
             (id, name, description, owners, contributors, tags, visibility, status, \
              northstar_md, northstar_version, northstar_created_by, northstar_modified_by, \
              northstar_created_at, northstar_modified_at, created_at, updated_at) \
             VALUES ($1,$2,$3,$4,'','','internal','active','',1,'','',NULL,NULL,$5,$5)",
        )
        .bind(&new_id)
        .bind(&new_name)
        .bind(mission_desc)
        .bind(&principal.subject)
        .bind(now)
        .execute(&state.db)
        .await
        {
            Ok(_) => { created_missions.push(new_id.clone()); new_id }
            Err(e) if e.to_string().contains("unique") || e.to_string().contains("duplicate") => {
                return (StatusCode::CONFLICT, Json(serde_json::json!({"detail": "Mission name already exists; provide target_mission_id to install into an existing mission"}))).into_response();
            }
            Err(e) => { tracing::error!("install_pack create_mission: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
        }
    };

    // 6. Create klusters (skip existing by name)
    let mut kluster_entries: Vec<(&str, &[u8])> = entries.iter()
        .filter(|(k, _)| k.starts_with("klusters/") && k.ends_with(".json"))
        .map(|(k, v)| (k.as_str(), v.as_slice()))
        .collect();
    kluster_entries.sort_by_key(|(k, _)| *k);

    for (_, content) in &kluster_entries {
        let k_spec: serde_json::Value = match serde_json::from_slice(content) {
            Ok(v) => v, Err(_) => continue,
        };
        let k_name = k_spec.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let k_desc = k_spec.get("description").and_then(|v| v.as_str()).unwrap_or("");
        if k_name.is_empty() { continue; }

        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM kluster WHERE mission_id=$1 AND name=$2)",
        )
        .bind(&mission_id)
        .bind(k_name)
        .fetch_one(&state.db)
        .await
        .unwrap_or(false);

        if !exists {
            let k_id = new_hash_id();
            let _ = sqlx::query(
                "INSERT INTO kluster (id, mission_id, name, description, owners, contributors, tags, status, \
                 workstream_md, workstream_version, workstream_created_by, workstream_modified_by, \
                 workstream_created_at, workstream_modified_at, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,'','','active','',1,'','',NULL,NULL,$6,$6)",
            )
            .bind(&k_id)
            .bind(&mission_id)
            .bind(k_name)
            .bind(k_desc)
            .bind(&principal.subject)
            .bind(now)
            .execute(&state.db)
            .await;
            created_klusters.push(k_id);
        }
    }

    // 7. Create skill bundles (skip existing by sha256 for this mission)
    let mut skill_entries: Vec<(&str, &[u8])> = entries.iter()
        .filter(|(k, _)| k.starts_with("skills/") && k.ends_with(".json"))
        .map(|(k, v)| (k.as_str(), v.as_slice()))
        .collect();
    skill_entries.sort_by_key(|(k, _)| *k);

    for (_, content) in &skill_entries {
        let s_spec: serde_json::Value = match serde_json::from_slice(content) {
            Ok(v) => v, Err(_) => continue,
        };
        let s_sha256 = s_spec.get("sha256").and_then(|v| v.as_str()).unwrap_or("");
        let s_tarball = s_spec.get("tarball_b64").and_then(|v| v.as_str()).unwrap_or("");
        let s_version = s_spec.get("version").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
        if s_sha256.is_empty() { continue; }

        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM skillbundle WHERE scope_type='mission' AND scope_id=$1 AND sha256=$2)",
        )
        .bind(&mission_id)
        .bind(s_sha256)
        .fetch_one(&state.db)
        .await
        .unwrap_or(false);

        if !exists {
            let sb_id = Uuid::new_v4().to_string();
            let size_bytes = STANDARD.decode(s_tarball).map(|b| b.len() as i32).unwrap_or(0);
            let _ = sqlx::query(
                "INSERT INTO skillbundle \
                 (id, scope_type, scope_id, mission_id, kluster_id, version, status, \
                  signature_alg, signing_key_id, signature, signature_verified, \
                  manifest_json, tarball_b64, sha256, size_bytes, created_by, created_at, updated_at) \
                 VALUES ($1,'mission',$2,$2,'', $3,'active','','','',false,'[]',$4,$5,$6,$7,$8,$8)",
            )
            .bind(&sb_id)
            .bind(&mission_id)
            .bind(s_version)
            .bind(s_tarball)
            .bind(s_sha256)
            .bind(size_bytes)
            .bind(&principal.subject)
            .bind(now)
            .execute(&state.db)
            .await;
            created_skills.push(sb_id);
        }
    }

    // 8. Create budget policies
    let mut budget_entries: Vec<(&str, &[u8])> = entries.iter()
        .filter(|(k, _)| k.starts_with("budgets/") && k.ends_with(".json"))
        .map(|(k, v)| (k.as_str(), v.as_slice()))
        .collect();
    budget_entries.sort_by_key(|(k, _)| *k);

    for (_, content) in &budget_entries {
        let b_spec: serde_json::Value = match serde_json::from_slice(content) {
            Ok(v) => v, Err(_) => continue,
        };
        let window_type = b_spec.get("window_type").and_then(|v| v.as_str()).unwrap_or("month");
        let hard_cap_cents = b_spec.get("hard_cap_cents").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let soft_cap_cents: Option<i32> = b_spec.get("soft_cap_cents").and_then(|v| v.as_i64()).map(|v| v as i32);
        let action_on_breach = b_spec.get("action_on_breach").and_then(|v| v.as_str()).unwrap_or("alert_only");

        let policy_id = Uuid::new_v4().to_string();
        let _ = sqlx::query(
            "INSERT INTO budgetpolicy \
             (id, owner_subject, scope_type, scope_id, window_type, hard_cap_cents, soft_cap_cents, \
              token_hard_cap, token_soft_cap, action_on_breach, active, created_at, updated_at) \
             VALUES ($1,$2,'mission',$3,$4,$5,$6,NULL,NULL,$7,true,$8,$8)",
        )
        .bind(&policy_id)
        .bind(&principal.subject)
        .bind(&mission_id)
        .bind(window_type)
        .bind(hard_cap_cents)
        .bind(soft_cap_cents)
        .bind(action_on_breach)
        .bind(now)
        .execute(&state.db)
        .await;
        created_budgets.push(policy_id);
    }

    Json(serde_json::json!({
        "mission_id": mission_id,
        "created": {
            "missions": created_missions,
            "klusters": created_klusters,
            "skills": created_skills,
            "budgets": created_budgets,
        }
    }))
    .into_response()
}
