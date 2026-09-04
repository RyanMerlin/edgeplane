use axum::{
    Json, Router,
    extract::{Query, State},
    response::IntoResponse,
    routing::get,
};
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/search/tasks", get(search_tasks))
        .route("/search/docs", get(search_docs))
        .route("/search/missions", get(search_missions))
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
    let limit = q.limit.unwrap_or(10).clamp(1, 50);
    let pattern = format!("%{}%", q.q.to_lowercase());

    let rows = sqlx::query(
        "SELECT t.id, t.title, t.description, t.status, t.mission_id \
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
                    // id is `character varying` post migration 0014 (task/meshtask
                    // unification) — was `i32`.
                    "id": r.get::<String, _>("id"),
                    "title": r.get::<String, _>("title"),
                    // description is nullable on the unified table (claimable
                    // rows never set it) — was NOT NULL on the legacy `task`.
                    "description": r.try_get::<Option<String>, _>("description").ok().flatten().unwrap_or_default(),
                    "status": r.get::<String, _>("status"),
                    "mission_id": r.get::<String, _>("mission_id"),
                })
            })
            .collect();
        return Json(serde_json::json!({"results": results})).into_response();
    }

    // Filter by readable domains
    let mission_ids: Vec<String> = rows
        .iter()
        .map(|r| r.get::<String, _>("mission_id"))
        .collect();
    if mission_ids.is_empty() {
        return Json(serde_json::json!({"results": []})).into_response();
    }

    let readable_task_ids = get_readable_task_ids(&state.db, &principal, &rows).await;

    let results: Vec<serde_json::Value> = rows
        .iter()
        .filter(|r| readable_task_ids.contains(&r.get::<String, _>("id")))
        .take(limit as usize)
        .map(|r| {
            serde_json::json!({
                "id": r.get::<String, _>("id"),
                "title": r.get::<String, _>("title"),
                "description": r.try_get::<Option<String>, _>("description").ok().flatten().unwrap_or_default(),
                "status": r.get::<String, _>("status"),
                "mission_id": r.get::<String, _>("mission_id"),
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
    let limit = q.limit.unwrap_or(10).clamp(1, 50);
    let pattern = format!("%{}%", q.q.to_lowercase());

    let rows = sqlx::query(
        "SELECT d.id, d.title, d.body, d.doc_type, d.status, d.mission_id \
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
                    "mission_id": r.get::<String, _>("mission_id"),
                })
            })
            .collect();
        return Json(serde_json::json!({"results": results})).into_response();
    }

    let readable_doc_ids = get_readable_doc_ids(&state.db, &principal, &rows).await;

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
                "mission_id": r.get::<String, _>("mission_id"),
            })
        })
        .collect();

    Json(serde_json::json!({"results": results})).into_response()
}

async fn search_missions(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<SearchQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(10).clamp(1, 50);
    let pattern = format!("%{}%", q.q.to_lowercase());

    let rows = if principal.is_admin {
        sqlx::query(
            "SELECT * FROM mission \
             WHERE LOWER(name) LIKE $1 OR LOWER(COALESCE(tags,'')) LIKE $1 \
             ORDER BY updated_at DESC LIMIT $2",
        )
        .bind(&pattern)
        .bind(limit)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default()
    } else {
        // Text-match candidates in SQL (overfetch limit*4), then apply the
        // exact-membership readability rule in Rust — see readable_mission_ids
        // for why LIKE-based membership is wrong (substring leak). Read the
        // JOINED domain's id (m.id AS d_domain_id), not the mission's raw FK, so
        // an orphaned FK (domain row gone) fails closed like the task/doc paths.
        let cand = sqlx::query(
            "SELECT k.*, m.id AS d_domain_id, m.visibility AS d_visibility, \
                    m.owners AS d_owners, m.contributors AS d_contributors \
             FROM mission k \
             LEFT JOIN domain m ON m.id = k.domain_id \
             WHERE (LOWER(k.name) LIKE $1 OR LOWER(COALESCE(k.tags,'')) LIKE $1) \
             ORDER BY k.updated_at DESC LIMIT $2",
        )
        .bind(&pattern)
        .bind(limit * 4)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

        cand.into_iter()
            .filter(|r| {
                let visibility: Option<String> = r.get("d_visibility");
                if visibility.as_deref() == Some("public") {
                    return true;
                }
                let domain_id: Option<String> = r.get("d_domain_id");
                match domain_id {
                    Some(did) => {
                        let owners: String =
                            r.get::<Option<String>, _>("d_owners").unwrap_or_default();
                        let contributors: String = r
                            .get::<Option<String>, _>("d_contributors")
                            .unwrap_or_default();
                        crate::auth::authorized_for(&did, &owners, &contributors, &principal)
                    }
                    None => false,
                }
            })
            .take(limit as usize)
            .collect::<Vec<_>>()
    };

    let results: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<String, _>("id"),
                "name": r.get::<String, _>("name"),
                "domain_id": r.get::<Option<String>, _>("domain_id"),
                "tags": r.get::<Option<String>, _>("tags"),
                "status": r.get::<String, _>("status"),
            })
        })
        .collect();

    Json(serde_json::json!({"results": results})).into_response()
}

/// Returns the readable-mission ids (of `mission_ids`) for `principal`, per the
/// exact-membership rule: public domain visibility, OR
/// `crate::auth::authorized_for` (admin/node/domain_scope/exact owner-or-contributor).
/// A mission with no domain (`domain_id` NULL) is not readable by a non-admin
/// (fail-closed — admins already short-circuit at the call sites).
async fn readable_mission_ids(
    db: &sqlx::PgPool,
    principal: &Principal,
    mission_ids: &[String],
) -> std::collections::HashSet<String> {
    if mission_ids.is_empty() {
        return std::collections::HashSet::new();
    }

    let cand = sqlx::query(
        "SELECT k.id AS mission_id, m.id AS domain_id, m.visibility, m.owners, m.contributors \
         FROM mission k LEFT JOIN domain m ON m.id = k.domain_id \
         WHERE k.id = ANY($1)",
    )
    .bind(mission_ids)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    cand.iter()
        .filter(|r| {
            let visibility: Option<String> = r.get("visibility");
            if visibility.as_deref() == Some("public") {
                return true;
            }
            let domain_id: Option<String> = r.get("domain_id");
            match domain_id {
                Some(did) => {
                    let owners: String = r.get::<Option<String>, _>("owners").unwrap_or_default();
                    let contributors: String = r
                        .get::<Option<String>, _>("contributors")
                        .unwrap_or_default();
                    crate::auth::authorized_for(&did, &owners, &contributors, principal)
                }
                None => false,
            }
        })
        .map(|r| r.get::<String, _>("mission_id"))
        .collect()
}

// Returns set of task ids readable by the given principal (via mission → domain
// exact owners/contributors membership, or public domain visibility).
async fn get_readable_task_ids(
    db: &sqlx::PgPool,
    principal: &Principal,
    rows: &[sqlx::postgres::PgRow],
) -> std::collections::HashSet<String> {
    let mission_ids: Vec<String> = rows
        .iter()
        .map(|r| r.get::<String, _>("mission_id"))
        .collect();
    if mission_ids.is_empty() {
        return std::collections::HashSet::new();
    }

    let readable_missions = readable_mission_ids(db, principal, &mission_ids).await;

    rows.iter()
        .filter(|r| readable_missions.contains(&r.get::<String, _>("mission_id")))
        .map(|r| r.get::<String, _>("id"))
        .collect()
}

async fn get_readable_doc_ids(
    db: &sqlx::PgPool,
    principal: &Principal,
    rows: &[sqlx::postgres::PgRow],
) -> std::collections::HashSet<i32> {
    let mission_ids: Vec<String> = rows
        .iter()
        .map(|r| r.get::<String, _>("mission_id"))
        .collect();
    if mission_ids.is_empty() {
        return std::collections::HashSet::new();
    }

    let readable_missions = readable_mission_ids(db, principal, &mission_ids).await;

    rows.iter()
        .filter(|r| readable_missions.contains(&r.get::<String, _>("mission_id")))
        .map(|r| r.get::<i32, _>("id"))
        .collect()
}
