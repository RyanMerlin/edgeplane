use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/family/members", get(list_members).post(create_member))
        .route("/family/members/{subject}", get(get_member).put(update_member))
        .route("/family/members/{subject}/access", get(check_access))
}

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Deserialize)]
struct MemberCreate {
    subject: String,
    display_name: String,
    age_group: String,
    goose_mode: Option<String>,
    model_allowlist: Option<Vec<String>>,
    token_daily_cap: Option<i64>,
    allowed_hours_start: Option<String>,
    allowed_hours_end: Option<String>,
    allowed_hours_tz: Option<String>,
    next_review_date: Option<String>,
}

#[derive(Deserialize)]
struct MemberUpdate {
    display_name: Option<String>,
    age_group: Option<String>,
    goose_mode: Option<String>,
    model_allowlist: Option<Vec<String>>,
    token_daily_cap: Option<i64>,
    allowed_hours_start: Option<String>,
    allowed_hours_end: Option<String>,
    allowed_hours_tz: Option<String>,
    next_review_date: Option<String>,
}

#[derive(Serialize)]
struct MemberRead {
    id: String,
    subject: String,
    display_name: String,
    age_group: String,
    goose_mode: String,
    model_allowlist: Vec<String>,
    token_daily_cap: Option<i64>,
    allowed_hours_start: Option<String>,
    allowed_hours_end: Option<String>,
    allowed_hours_tz: String,
    next_review_date: Option<String>,
    created_at: chrono::NaiveDateTime,
    updated_at: chrono::NaiveDateTime,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn row_to_member(row: &sqlx::postgres::PgRow) -> MemberRead {
    let allowlist_json: String = row.try_get("model_allowlist_json").unwrap_or_default();
    let model_allowlist: Vec<String> =
        serde_json::from_str(&allowlist_json).unwrap_or_default();
    MemberRead {
        id: row.get("id"),
        subject: row.get("subject"),
        display_name: row.get("display_name"),
        age_group: row.get("age_group"),
        goose_mode: row.get("goose_mode"),
        model_allowlist,
        token_daily_cap: row.get("token_daily_cap"),
        allowed_hours_start: row.get("allowed_hours_start"),
        allowed_hours_end: row.get("allowed_hours_end"),
        allowed_hours_tz: row.try_get("allowed_hours_tz").unwrap_or_else(|_| "America/New_York".into()),
        next_review_date: row.get("next_review_date"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn not_found() -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"detail": "member not found"})),
    )
        .into_response()
}

fn check_time_window(
    start: Option<&str>,
    end: Option<&str>,
    tz_name: &str,
) -> (bool, String) {
    let (Some(s), Some(e)) = (start, end) else {
        return (true, "no time restriction".into());
    };
    let Ok(start_t) = s.parse::<chrono::NaiveTime>() else {
        return (true, "time check skipped (parse error)".into());
    };
    let Ok(end_t) = e.parse::<chrono::NaiveTime>() else {
        return (true, "time check skipped (parse error)".into());
    };
    // Parse timezone — fall back to UTC if unknown
    let now_naive = if let Ok(tz) = tz_name.parse::<chrono_tz::Tz>() {
        Utc::now().with_timezone(&tz).naive_local().time()
    } else {
        Utc::now().naive_utc().time()
    };
    if start_t <= now_naive && now_naive <= end_t {
        (true, "within allowed hours".into())
    } else {
        (false, format!("outside allowed hours ({s}–{e} {tz_name})"))
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn list_members(
    State(state): State<Arc<AppState>>,
    principal: Principal,
) -> impl IntoResponse {
    if !principal.is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "admin required"})),
        )
            .into_response();
    }
    match sqlx::query("SELECT * FROM familymember ORDER BY created_at ASC")
        .fetch_all(&state.db)
        .await
    {
        Ok(rows) => Json(rows.iter().map(row_to_member).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_members: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_member(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(subject): Path<String>,
) -> impl IntoResponse {
    // Self-read allowed; admin can read anyone
    if principal.subject != subject && !principal.is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "forbidden"})),
        )
            .into_response();
    }
    match sqlx::query("SELECT * FROM familymember WHERE subject=$1")
        .bind(&subject)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(row)) => Json(row_to_member(&row)).into_response(),
        Ok(None) => not_found(),
        Err(e) => {
            tracing::error!("get_member: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn create_member(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<MemberCreate>,
) -> impl IntoResponse {
    if !principal.is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "admin required"})),
        )
            .into_response();
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();
    let goose_mode = payload.goose_mode.unwrap_or_else(|| "chat".into());
    let allowlist_json = serde_json::to_string(
        &payload.model_allowlist.unwrap_or_default(),
    )
    .unwrap_or_else(|_| "[]".into());
    let tz = payload.allowed_hours_tz.unwrap_or_else(|| "America/New_York".into());

    let result = sqlx::query(
        "INSERT INTO familymember \
         (id, subject, display_name, age_group, goose_mode, model_allowlist_json, \
          token_daily_cap, allowed_hours_start, allowed_hours_end, allowed_hours_tz, \
          next_review_date, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$12) RETURNING *",
    )
    .bind(&id)
    .bind(&payload.subject)
    .bind(&payload.display_name)
    .bind(&payload.age_group)
    .bind(&goose_mode)
    .bind(&allowlist_json)
    .bind(payload.token_daily_cap)
    .bind(&payload.allowed_hours_start)
    .bind(&payload.allowed_hours_end)
    .bind(&tz)
    .bind(&payload.next_review_date)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => (StatusCode::CREATED, Json(row_to_member(&row))).into_response(),
        Err(e) if e.to_string().contains("unique") || e.to_string().contains("duplicate") => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"detail": "member already exists"})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("create_member: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn update_member(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(subject): Path<String>,
    Json(payload): Json<MemberUpdate>,
) -> impl IntoResponse {
    if !principal.is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "admin required"})),
        )
            .into_response();
    }
    let existing = sqlx::query("SELECT * FROM familymember WHERE subject=$1")
        .bind(&subject)
        .fetch_optional(&state.db)
        .await;
    let row = match existing {
        Ok(Some(r)) => r,
        Ok(None) => return not_found(),
        Err(e) => {
            tracing::error!("update_member fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let current = row_to_member(&row);
    let display_name = payload.display_name.unwrap_or(current.display_name);
    let age_group = payload.age_group.unwrap_or(current.age_group);
    let goose_mode = payload.goose_mode.unwrap_or(current.goose_mode);
    let allowlist_json = payload
        .model_allowlist
        .map(|v| serde_json::to_string(&v).unwrap_or_else(|_| "[]".into()))
        .unwrap_or_else(|| serde_json::to_string(&current.model_allowlist).unwrap_or_else(|_| "[]".into()));
    let token_daily_cap = if payload.token_daily_cap.is_some() {
        payload.token_daily_cap
    } else {
        current.token_daily_cap
    };
    let allowed_hours_start = payload.allowed_hours_start.or(current.allowed_hours_start);
    let allowed_hours_end = payload.allowed_hours_end.or(current.allowed_hours_end);
    let allowed_hours_tz = payload.allowed_hours_tz.unwrap_or(current.allowed_hours_tz);
    let next_review_date = payload.next_review_date.or(current.next_review_date);
    let now = Utc::now().naive_utc();

    match sqlx::query(
        "UPDATE familymember SET \
         display_name=$2, age_group=$3, goose_mode=$4, model_allowlist_json=$5, \
         token_daily_cap=$6, allowed_hours_start=$7, allowed_hours_end=$8, \
         allowed_hours_tz=$9, next_review_date=$10, updated_at=$11 \
         WHERE subject=$1 RETURNING *",
    )
    .bind(&subject)
    .bind(&display_name)
    .bind(&age_group)
    .bind(&goose_mode)
    .bind(&allowlist_json)
    .bind(token_daily_cap)
    .bind(&allowed_hours_start)
    .bind(&allowed_hours_end)
    .bind(&allowed_hours_tz)
    .bind(&next_review_date)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => Json(row_to_member(&row)).into_response(),
        Err(e) => {
            tracing::error!("update_member: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn check_access(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(subject): Path<String>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT * FROM familymember WHERE subject=$1")
        .bind(&subject)
        .fetch_optional(&state.db)
        .await;
    match row {
        Ok(Some(r)) => {
            let start: Option<String> = r.get("allowed_hours_start");
            let end: Option<String> = r.get("allowed_hours_end");
            let tz: String = r.try_get("allowed_hours_tz").unwrap_or_else(|_| "America/New_York".into());
            let (allowed, reason) = check_time_window(
                start.as_deref(),
                end.as_deref(),
                &tz,
            );
            Json(serde_json::json!({"allowed": allowed, "reason": reason})).into_response()
        }
        Ok(None) => not_found(),
        Err(e) => {
            tracing::error!("check_access: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
