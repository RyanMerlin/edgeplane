use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/ops/backups", post(trigger_backup).get(list_backups))
        .route("/ops/secrets/status", get(get_secrets_status))
        .route("/ops/secrets/bootstrap", post(post_secrets_bootstrap))
        .route("/ops/secrets/rotate", post(post_secrets_rotate))
}

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Deserialize)]
struct BackupRequest {
    target: Option<String>,
    reason: Option<String>,
}

#[derive(Deserialize)]
struct SecretsBootstrapRequest {
    profile: Option<String>,
    provider: Option<String>,
    keep_existing: Option<bool>,
    infisical_project_id: Option<String>,
    infisical_env: Option<String>,
    infisical_path: Option<String>,
}

#[derive(Deserialize)]
struct SecretsRotateRequest {
    profile: Option<String>,
    name: Option<String>,
    value: Option<String>,
    generator: Option<String>,
}

#[derive(Serialize, Clone)]
struct BackupRecord {
    id: String,
    target: String,
    reason: Option<String>,
    triggered_by: String,
    status: String,
    created_at: String,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn backup_dir() -> String {
    std::env::var("MC_BACKUP_DIR").unwrap_or_else(|_| "/data/backups".to_string())
}

fn backup_file() -> String {
    format!("{}/records.json", backup_dir())
}

async fn read_backup_records() -> Vec<serde_json::Value> {
    match tokio::fs::read_to_string(backup_file()).await {
        Ok(contents) => serde_json::from_str::<Vec<serde_json::Value>>(&contents).unwrap_or_default(),
        Err(_) => vec![],
    }
}

async fn write_backup_records(records: &[serde_json::Value]) -> std::io::Result<()> {
    let dir = backup_dir();
    tokio::fs::create_dir_all(&dir).await?;
    let path = backup_file();
    let json = serde_json::to_string_pretty(records).unwrap_or_else(|_| "[]".into());
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .await?;
    file.write_all(json.as_bytes()).await?;
    file.flush().await?;
    Ok(())
}

fn mc_home() -> std::path::PathBuf {
    let val = std::env::var("MC_HOME").unwrap_or_default();
    if !val.is_empty() {
        return std::path::PathBuf::from(val);
    }
    dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/root")).join(".mc")
}

fn profile_secrets_path(profile: &str) -> std::path::PathBuf {
    mc_home().join("profiles").join(profile).join("secrets.json")
}

fn load_profile_data(path: &std::path::Path) -> serde_json::Value {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or(serde_json::json!({})),
        Err(_) => serde_json::json!({}),
    }
}

async fn save_profile_data(path: &std::path::Path, data: &serde_json::Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let s = serde_json::to_string_pretty(data).unwrap_or_else(|_| "{}".to_string());
    tokio::fs::write(path, s.as_bytes()).await
}

fn build_secret_ref(
    name: &str,
    provider: &str,
    project_id: Option<&str>,
    env: Option<&str>,
    path: Option<&str>,
) -> String {
    if provider == "infisical" {
        let mut parts = Vec::new();
        if let Some(p) = project_id { parts.push(format!("projectId={p}")); }
        if let Some(e) = env { parts.push(format!("env={e}")); }
        if let Some(p) = path { parts.push(format!("path={p}")); }
        let query = if parts.is_empty() { String::new() } else { format!("?{}", parts.join("&")) };
        format!("secret://infisical/{name}{query}")
    } else {
        format!("secret://env/{name}")
    }
}

fn generate_secret(generator: &str) -> String {
    use base64::Engine;
    use rand::RngCore;
    if generator == "hex" {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        hex::encode(bytes)
    } else {
        let mut bytes = [0u8; 48];
        rand::thread_rng().fill_bytes(&mut bytes);
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }
}

fn require_admin(principal: &Principal) -> Option<axum::response::Response> {
    if !principal.is_admin {
        Some(
            (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"detail": "admin_required"})),
            )
                .into_response(),
        )
    } else {
        None
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn trigger_backup(
    State(_state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<BackupRequest>,
) -> impl IntoResponse {
    if let Some(r) = require_admin(&principal) {
        return r;
    }

    let backup_id = hex::encode(Uuid::new_v4().as_bytes());
    let now = Utc::now();
    let created_at = now.to_rfc3339();

    let record = BackupRecord {
        id: backup_id.clone(),
        target: payload.target.clone().unwrap_or_else(|| "default".to_string()),
        reason: payload.reason.clone(),
        triggered_by: principal.subject.clone(),
        status: "triggered".to_string(),
        created_at: created_at.clone(),
    };

    // Append to records file
    let mut records = read_backup_records().await;
    records.push(serde_json::json!({
        "id": record.id,
        "target": record.target,
        "reason": record.reason,
        "triggered_by": record.triggered_by,
        "status": record.status,
        "created_at": record.created_at,
    }));

    if let Err(e) = write_backup_records(&records).await {
        tracing::error!("trigger_backup: failed to write records.json: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok": false, "error": "failed to persist backup record"})),
        )
            .into_response();
    }

    // Optionally run the backup script
    if let Ok(script) = std::env::var("MC_BACKUP_SCRIPT") {
        let id_clone = backup_id.clone();
        tokio::spawn(async move {
            let result = tokio::process::Command::new(&script)
                .env("BACKUP_ID", &id_clone)
                .status()
                .await;
            match result {
                Ok(status) => tracing::info!("backup script exited: {status}"),
                Err(e) => tracing::error!("backup script failed to launch: {e}"),
            }
        });
    }

    Json(serde_json::json!({
        "ok": true,
        "backup": {
            "id": record.id,
            "target": record.target,
            "reason": record.reason,
            "triggered_by": record.triggered_by,
            "status": record.status,
            "created_at": record.created_at,
        }
    }))
    .into_response()
}

async fn list_backups(
    State(_state): State<Arc<AppState>>,
    principal: Principal,
) -> impl IntoResponse {
    if let Some(r) = require_admin(&principal) {
        return r;
    }

    let records = read_backup_records().await;
    Json(serde_json::json!({"backups": records})).into_response()
}

async fn get_secrets_status(
    State(_state): State<Arc<AppState>>,
    principal: Principal,
) -> impl IntoResponse {
    if let Some(r) = require_admin(&principal) {
        return r;
    }
    let profile_name = {
        let p = std::env::var("MC_SECRETS_PROFILE").unwrap_or_default();
        if p.trim().is_empty() {
            std::env::var("MC_AGENT_PROFILE").unwrap_or_else(|_| "default".into())
        } else {
            p
        }
    };
    let profile_name = if profile_name.trim().is_empty() { "default".to_string() } else { profile_name.trim().to_string() };
    let path = profile_secrets_path(&profile_name);
    let provider_env = {
        let raw = std::env::var("MC_SECRETS_PROVIDER").unwrap_or_default();
        if raw.trim().eq_ignore_ascii_case("infisical") { "infisical" } else { "env" }
    };
    let profile_exists = path.exists();
    let data = load_profile_data(&path);
    let refs_count = data.get("refs").and_then(|v| v.as_object()).map(|m| m.len()).unwrap_or(0);
    let provider_profile = data.get("provider").and_then(|v| v.as_str()).map(|s| s.to_string());
    let infisical = serde_json::json!({
        "project_id": data.get("infisical_project_id"),
        "env": data.get("infisical_env"),
        "path": data.get("infisical_path"),
    });
    Json(serde_json::json!({
        "secrets": {
            "provider_env": provider_env,
            "provider_profile": provider_profile,
            "effective_profile": profile_name,
            "profile_path": path.to_string_lossy(),
            "profile_exists": profile_exists,
            "refs_count": refs_count,
            "infisical": infisical,
        }
    })).into_response()
}

async fn post_secrets_bootstrap(
    State(_state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<SecretsBootstrapRequest>,
) -> impl IntoResponse {
    if let Some(r) = require_admin(&principal) {
        return r;
    }
    let profile_name = payload.profile
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "default".into());
    let provider_raw = payload.provider
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "env".into());
    let provider = if provider_raw.trim().eq_ignore_ascii_case("infisical") { "infisical" } else { "env" };
    if provider != "env" && provider != "infisical" {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"ok": false, "error": "provider must be env or infisical"}))).into_response();
    }
    let keep_existing = payload.keep_existing.unwrap_or(true);
    let project_id = payload.infisical_project_id.filter(|s| !s.is_empty());
    let infisical_env = payload.infisical_env.filter(|s| !s.is_empty());
    let infisical_path = payload.infisical_path.filter(|s| !s.is_empty());

    let path = profile_secrets_path(&profile_name);
    let existing = load_profile_data(&path);

    let mut refs: serde_json::Map<String, serde_json::Value> = existing
        .get("refs")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    const DEFAULT_NAMES: &[&str] = &[
        "MC_TOKEN", "MQTT_PASSWORD", "POSTGRES_PASSWORD",
        "MC_OBJECT_STORAGE_ACCESS_KEY", "MC_OBJECT_STORAGE_ACCESS_SECRET",
    ];
    for &name in DEFAULT_NAMES {
        if keep_existing && refs.contains_key(name) { continue; }
        let ref_val = build_secret_ref(name, provider, project_id.as_deref(), infisical_env.as_deref(), infisical_path.as_deref());
        refs.insert(name.to_string(), serde_json::json!(ref_val));
    }

    let refs_count = refs.len();
    let mut new_data = serde_json::json!({
        "refs": refs,
        "provider": provider,
    });
    if provider == "infisical" {
        new_data["infisical_project_id"] = serde_json::json!(project_id);
        new_data["infisical_env"] = serde_json::json!(infisical_env);
        new_data["infisical_path"] = serde_json::json!(infisical_path);
    }

    if let Err(e) = save_profile_data(&path, &new_data).await {
        tracing::error!("secrets_bootstrap save: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Json(serde_json::json!({
        "ok": true,
        "result": {
            "profile": profile_name,
            "provider": provider,
            "path": path.to_string_lossy(),
            "refs_count": refs_count,
        }
    })).into_response()
}

async fn post_secrets_rotate(
    State(_state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<SecretsRotateRequest>,
) -> impl IntoResponse {
    if let Some(r) = require_admin(&principal) {
        return r;
    }
    let profile_name = payload.profile
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "default".into());
    let secret_name = match payload.name.filter(|s| !s.trim().is_empty()) {
        Some(n) => n,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"ok": false, "error": "name is required"}))).into_response(),
    };
    let generator = payload.generator.filter(|s| !s.is_empty()).unwrap_or_else(|| "token".into());

    let path = profile_secrets_path(&profile_name);
    let data = load_profile_data(&path);

    let ref_val = data.get("refs")
        .and_then(|v| v.as_object())
        .and_then(|r| r.get(&secret_name))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let ref_val = match ref_val {
        Some(r) => r,
        None => return (StatusCode::CONFLICT, Json(serde_json::json!({"ok": false, "error": format!("Secret '{}' is not mapped in profile '{}'", secret_name, profile_name)}))).into_response(),
    };

    let next_value = payload.value.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| generate_secret(&generator));

    let (provider, target_name) = if let Some(rest) = ref_val.strip_prefix("secret://infisical/") {
        ("infisical", rest.split('?').next().unwrap_or(&secret_name).to_string())
    } else if let Some(rest) = ref_val.strip_prefix("secret://env/") {
        ("env", rest.split('?').next().unwrap_or(&secret_name).to_string())
    } else {
        ("env", secret_name.clone())
    };

    if provider == "env" {
        // SAFETY: single-threaded context during startup config; best-effort env update
        unsafe { std::env::set_var(&target_name, &next_value) };
    } else {
        let cli = std::env::var("MC_SECRETS_INFISICAL_CLI_BIN").unwrap_or_else(|_| "infisical".into());
        let project_id = data.get("infisical_project_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let infisical_env = data.get("infisical_env").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let infisical_path_val = data.get("infisical_path").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let mut cmd = std::process::Command::new(&cli);
        cmd.args(["secrets", "set", &target_name, &next_value]);
        if !project_id.is_empty() { cmd.args(["--projectId", &project_id]); }
        if !infisical_env.is_empty() { cmd.args(["--env", &infisical_env]); }
        if !infisical_path_val.is_empty() { cmd.args(["--path", &infisical_path_val]); }
        match cmd.output() {
            Ok(out) if out.status.success() => {}
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr).to_string();
                return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"ok": false, "error": err}))).into_response();
            }
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"ok": false, "error": e.to_string()}))).into_response(),
        }
    }

    Json(serde_json::json!({
        "ok": true,
        "result": {
            "profile": profile_name,
            "name": secret_name,
            "provider": provider,
            "reference": ref_val,
            "updated": true,
        }
    })).into_response()
}
