use axum::{
    extract::{Query, State},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/search/tasks", get(search_tasks))
        .route("/search/docs", get(search_docs))
        .route("/search/klusters", get(search_klusters))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    limit: Option<i64>,
}

async fn search_tasks(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<SearchQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(10).min(50).max(1);
    let pattern = format!("%{}%", q.q.to_lowercase());

    let rows = sqlx::query(
        "SELECT t.id, t.title, t.description, t.status, t.kluster_id \
         FROM task t \
         WHERE LOWER(t.title) LIKE $1 OR LOWER(t.description) LIKE $1 \
         ORDER BY t.updated_at DESC LIMIT $2",
    )
    .bind(&pattern)
    .bind(limit * 4)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    if principal.is_admin {
        let results: Vec<serde_json::Value> = rows
            .iter()
            .take(limit as usize)
            .map(|r| {
                serde_json::json!({
                    "id": r.get::<i32, _>("id"),
                    "title": r.get::<String, _>("title"),
                    "description": r.get::<String, _>("description"),
                    "status": r.get::<String, _>("status"),
                    "kluster_id": r.get::<String, _>("kluster_id"),
                })
            })
            .collect();
        return Json(serde_json::json!({"results": results})).into_response();
    }

    // Filter by readable missions
    let kluster_ids: Vec<String> = rows.iter().map(|r| r.get::<String, _>("kluster_id")).collect();
    if kluster_ids.is_empty() {
        return Json(serde_json::json!({"results": []})).into_response();
    }

    let readable_task_ids = get_readable_task_ids(&state.db, &principal.subject, &rows).await;

    let results: Vec<serde_json::Value> = rows
        .iter()
        .filter(|r| readable_task_ids.contains(&r.get::<i32, _>("id")))
        .take(limit as usize)
        .map(|r| {
            serde_json::json!({
                "id": r.get::<i32, _>("id"),
                "title": r.get::<String, _>("title"),
                "description": r.get::<String, _>("description"),
                "status": r.get::<String, _>("status"),
                "kluster_id": r.get::<String, _>("kluster_id"),
            })
        })
        .collect();

    Json(serde_json::json!({"results": results})).into_response()
}

async fn search_docs(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<SearchQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(10).min(50).max(1);
    let pattern = format!("%{}%", q.q.to_lowercase());

    let rows = sqlx::query(
        "SELECT d.id, d.title, d.body, d.doc_type, d.status, d.kluster_id \
         FROM doc d \
         WHERE LOWER(d.title) LIKE $1 OR LOWER(d.body) LIKE $1 \
         ORDER BY d.updated_at DESC LIMIT $2",
    )
    .bind(&pattern)
    .bind(limit * 4)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    if principal.is_admin {
        let results: Vec<serde_json::Value> = rows
            .iter()
            .take(limit as usize)
            .map(|r| {
                serde_json::json!({
                    "id": r.get::<i32, _>("id"),
                    "title": r.get::<String, _>("title"),
                    "doc_type": r.get::<String, _>("doc_type"),
                    "status": r.get::<String, _>("status"),
                    "kluster_id": r.get::<String, _>("kluster_id"),
                })
            })
            .collect();
        return Json(serde_json::json!({"results": results})).into_response();
    }

    let readable_doc_ids = get_readable_doc_ids(&state.db, &principal.subject, &rows).await;

    let results: Vec<serde_json::Value> = rows
        .iter()
        .filter(|r| readable_doc_ids.contains(&r.get::<i32, _>("id")))
        .take(limit as usize)
        .map(|r| {
            serde_json::json!({
                "id": r.get::<i32, _>("id"),
                "title": r.get::<String, _>("title"),
                "doc_type": r.get::<String, _>("doc_type"),
                "status": r.get::<String, _>("status"),
                "kluster_id": r.get::<String, _>("kluster_id"),
            })
        })
        .collect();

    Json(serde_json::json!({"results": results})).into_response()
}

async fn search_klusters(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<SearchQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(10).min(50).max(1);
    let pattern = format!("%{}%", q.q.to_lowercase());

    let rows = if principal.is_admin {
        sqlx::query(
            "SELECT * FROM kluster \
             WHERE LOWER(name) LIKE $1 OR LOWER(COALESCE(tags,'')) LIKE $1 \
             ORDER BY updated_at DESC LIMIT $2",
        )
        .bind(&pattern)
        .bind(limit)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default()
    } else {
        sqlx::query(
            "SELECT k.* FROM kluster k \
             LEFT JOIN mission m ON m.id = k.mission_id \
             WHERE (LOWER(k.name) LIKE $1 OR LOWER(COALESCE(k.tags,'')) LIKE $1) \
               AND (m.visibility='public' \
                    OR LOWER(m.owners) LIKE $2 \
                    OR LOWER(m.contributors) LIKE $2 \
                    OR EXISTS(SELECT 1 FROM missionrolemembership mrm WHERE mrm.mission_id=m.id AND mrm.subject=$3)) \
             ORDER BY k.updated_at DESC LIMIT $4",
        )
        .bind(&pattern)
        .bind(format!("%{}%", principal.subject.to_lowercase()))
        .bind(&principal.subject)
        .bind(limit)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default()
    };

    let results: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<String, _>("id"),
                "name": r.get::<String, _>("name"),
                "mission_id": r.get::<Option<String>, _>("mission_id"),
                "tags": r.get::<Option<String>, _>("tags"),
                "status": r.get::<String, _>("status"),
            })
        })
        .collect();

    Json(serde_json::json!({"results": results})).into_response()
}

// Returns set of task ids readable by the given subject (via kluster → mission membership)
async fn get_readable_task_ids(
    db: &sqlx::PgPool,
    subject: &str,
    rows: &[sqlx::postgres::PgRow],
) -> std::collections::HashSet<i32> {
    let kluster_ids: Vec<String> = rows.iter().map(|r| r.get::<String, _>("kluster_id")).collect();
    if kluster_ids.is_empty() {
        return std::collections::HashSet::new();
    }
    let subject_lower = subject.to_lowercase();
    let like_pat = format!("%{}%", subject_lower);

    let readable_klusters: Vec<String> = sqlx::query(
        "SELECT k.id FROM kluster k \
         LEFT JOIN mission m ON m.id = k.mission_id \
         WHERE k.id = ANY($1) \
           AND (m.visibility='public' \
                OR LOWER(m.owners) LIKE $2 \
                OR LOWER(m.contributors) LIKE $2 \
                OR EXISTS(SELECT 1 FROM missionrolemembership mrm WHERE mrm.mission_id=m.id AND mrm.subject=$3))",
    )
    .bind(&kluster_ids)
    .bind(&like_pat)
    .bind(subject)
    .fetch_all(db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|r| r.get::<String, _>("id"))
    .collect();

    let readable_set: std::collections::HashSet<String> = readable_klusters.into_iter().collect();

    rows.iter()
        .filter(|r| readable_set.contains(&r.get::<String, _>("kluster_id")))
        .map(|r| r.get::<i32, _>("id"))
        .collect()
}

async fn get_readable_doc_ids(
    db: &sqlx::PgPool,
    subject: &str,
    rows: &[sqlx::postgres::PgRow],
) -> std::collections::HashSet<i32> {
    let kluster_ids: Vec<String> = rows.iter().map(|r| r.get::<String, _>("kluster_id")).collect();
    if kluster_ids.is_empty() {
        return std::collections::HashSet::new();
    }
    let subject_lower = subject.to_lowercase();
    let like_pat = format!("%{}%", subject_lower);

    let readable_klusters: Vec<String> = sqlx::query(
        "SELECT k.id FROM kluster k \
         LEFT JOIN mission m ON m.id = k.mission_id \
         WHERE k.id = ANY($1) \
           AND (m.visibility='public' \
                OR LOWER(m.owners) LIKE $2 \
                OR LOWER(m.contributors) LIKE $2 \
                OR EXISTS(SELECT 1 FROM missionrolemembership mrm WHERE mrm.mission_id=m.id AND mrm.subject=$3))",
    )
    .bind(&kluster_ids)
    .bind(&like_pat)
    .bind(subject)
    .fetch_all(db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|r| r.get::<String, _>("id"))
    .collect();

    let readable_set: std::collections::HashSet<String> = readable_klusters.into_iter().collect();

    rows.iter()
        .filter(|r| readable_set.contains(&r.get::<String, _>("kluster_id")))
        .map(|r| r.get::<i32, _>("id"))
        .collect()
}
