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
use std::collections::HashMap;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/explorer/tree", get(explorer_tree))
        .route("/explorer/node/{node_type}/{node_id}", get(explorer_node))
}

// --- row helpers ---

fn row_to_domain(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    json!({
        "id": row.get::<String, _>("id"),
        "name": row.get::<String, _>("name"),
        "description": row.get::<String, _>("description"),
        "status": row.get::<String, _>("status"),
        "visibility": row.get::<String, _>("visibility"),
        "owners": row.get::<String, _>("owners"),
        "contributors": row.get::<String, _>("contributors"),
        "tags": row.get::<Option<String>, _>("tags"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

fn row_to_mission(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    json!({
        "id": row.get::<String, _>("id"),
        "domain_id": row.get::<Option<String>, _>("domain_id"),
        "name": row.get::<String, _>("name"),
        "description": row.get::<String, _>("description"),
        "status": row.get::<String, _>("status"),
        "owners": row.get::<String, _>("owners"),
        "tags": row.get::<Option<String>, _>("tags"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
    })
}

fn row_to_task(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    json!({
        "id": row.get::<i32, _>("id"),
        "public_id": row.get::<String, _>("public_id"),
        "mission_id": row.get::<String, _>("mission_id"),
        "title": row.get::<String, _>("title"),
        "description": row.get::<String, _>("description"),
        "status": row.get::<String, _>("status"),
        "owner": row.get::<String, _>("owner"),
        "contributors": row.get::<String, _>("contributors"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
    })
}

// --- access helpers ---

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
        let in_list =
            |s: &str| s.split(',').map(|x| x.trim().to_lowercase()).any(|x| x == sub);
        if in_list(&owners) || in_list(&contributors) {
            return true;
        }
    }
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM domainrolemembership WHERE domain_id=$1 AND subject=$2)",
    )
    .bind(domain_id)
    .bind(&principal.subject)
    .fetch_one(db)
    .await
    .unwrap_or(false)
}

// --- text filter ---

fn matches_query(needle: &str, values: &[Option<&str>]) -> bool {
    for v in values {
        if let Some(s) = v {
            if s.to_lowercase().contains(needle) {
                return true;
            }
        }
    }
    false
}

// --- query structs ---

#[derive(Deserialize)]
struct TreeQuery {
    domain_id: Option<String>,
    status: Option<String>,
    q: Option<String>,
    limit_tasks_per_cluster: Option<i64>,
    limit_missions: Option<i64>,
}

#[derive(Deserialize)]
struct NodeQuery {
    limit_tasks: Option<i64>,
}

// --- /explorer/tree ---

async fn explorer_tree(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(params): Query<TreeQuery>,
) -> impl IntoResponse {
    let needle = params.q.as_deref().unwrap_or("").trim().to_lowercase();
    let limit_tasks_per_cluster = params
        .limit_tasks_per_cluster
        .unwrap_or(5)
        .min(50)
        .max(1) as usize;
    let limit_missions = params.limit_missions.unwrap_or(100).min(200).max(1);

    // --- fetch domains ---
    let domain_rows = {
        let q = match &params.domain_id {
            Some(mid) => sqlx::query(
                "SELECT * FROM domain WHERE id=$1 ORDER BY updated_at DESC",
            )
            .bind(mid)
            .fetch_all(&state.db)
            .await,
            None => sqlx::query("SELECT * FROM domain ORDER BY updated_at DESC")
                .fetch_all(&state.db)
                .await,
        };
        match q {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!("explorer_tree fetch domains: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    };

    // Build set of readable domain IDs for non-admins
    let readable_domain_ids: std::collections::HashSet<String> = if principal.is_admin {
        domain_rows
            .iter()
            .map(|r| r.get::<String, _>("id"))
            .collect()
    } else {
        let mut ids = std::collections::HashSet::new();
        for row in &domain_rows {
            let id: String = row.get("id");
            if can_read_domain(&state.db, &principal, &id).await {
                ids.insert(id);
            }
        }
        ids
    };

    // Filter domains to readable
    let domains: Vec<&sqlx::postgres::PgRow> = domain_rows
        .iter()
        .filter(|r| {
            let id: String = r.get("id");
            readable_domain_ids.contains(&id)
        })
        .collect();

    // --- fetch missions ---
    let mission_rows = {
        let q = match &params.domain_id {
            Some(mid) => sqlx::query(
                "SELECT * FROM mission WHERE domain_id=$1 ORDER BY updated_at DESC LIMIT $2",
            )
            .bind(mid)
            .bind(limit_missions)
            .fetch_all(&state.db)
            .await,
            None => sqlx::query(
                "SELECT * FROM mission ORDER BY updated_at DESC LIMIT $1",
            )
            .bind(limit_missions)
            .fetch_all(&state.db)
            .await,
        };
        match q {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!("explorer_tree fetch missions: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    };

    // Filter missions to readable domains (non-admins: mission must be in a readable domain)
    let missions: Vec<&sqlx::postgres::PgRow> = mission_rows
        .iter()
        .filter(|r| {
            if principal.is_admin {
                return true;
            }
            let mid: Option<String> = r.try_get("domain_id").ok().and_then(|v: Option<String>| v);
            match mid {
                Some(mid) => readable_domain_ids.contains(&mid),
                None => false,
            }
        })
        .collect();

    // Collect mission IDs for task fetch
    let mission_ids: Vec<String> = missions.iter().map(|r| r.get::<String, _>("id")).collect();

    // --- fetch tasks ---
    let task_rows: Vec<sqlx::postgres::PgRow> = if mission_ids.is_empty() {
        vec![]
    } else {
        // Build parameterized IN clause
        let placeholders: String = mission_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");

        let (query_str, has_status) = match &params.status {
            Some(_) => (
                format!(
                    "SELECT * FROM task WHERE mission_id IN ({}) AND status=${} \
                     ORDER BY updated_at DESC",
                    placeholders,
                    mission_ids.len() + 1
                ),
                true,
            ),
            None => (
                format!(
                    "SELECT * FROM task WHERE mission_id IN ({}) ORDER BY updated_at DESC",
                    placeholders
                ),
                false,
            ),
        };

        let mut q_builder = sqlx::query(&query_str);
        for kid in &mission_ids {
            q_builder = q_builder.bind(kid);
        }
        if has_status {
            q_builder = q_builder.bind(params.status.as_deref().unwrap_or(""));
        }

        match q_builder.fetch_all(&state.db).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!("explorer_tree fetch tasks: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    };

    // --- group tasks by mission ---
    let mut tasks_by_mission: HashMap<String, Vec<&sqlx::postgres::PgRow>> = HashMap::new();
    for row in &task_rows {
        let kid: String = row.get("mission_id");
        tasks_by_mission.entry(kid).or_default().push(row);
    }

    // --- group missions by domain ---
    let mut missions_by_domain: HashMap<Option<String>, Vec<&sqlx::postgres::PgRow>> =
        HashMap::new();
    for row in &missions {
        let mid: Option<String> = row.try_get("domain_id").ok().and_then(|v: Option<String>| v);
        missions_by_domain.entry(mid).or_default().push(row);
    }

    // --- build domain summaries with text filter ---
    let mut domain_summaries: Vec<serde_json::Value> = Vec::new();
    let mut included_mission_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for domain_row in &domains {
        let domain_id_val: String = domain_row.get("id");
        let domain_name: String = domain_row.get("name");
        let domain_desc: String = domain_row.get("description");
        let domain_owners: String = domain_row.get("owners");
        let domain_tags: Option<String> = domain_row.try_get("tags").ok().and_then(|v: Option<String>| v);

        let domain_match = if needle.is_empty() {
            true
        } else {
            matches_query(
                &needle,
                &[
                    Some(domain_name.as_str()),
                    Some(domain_desc.as_str()),
                    Some(domain_owners.as_str()),
                    domain_tags.as_deref(),
                ],
            )
        };

        let empty_vec: Vec<&sqlx::postgres::PgRow> = vec![];
        let domain_missions = missions_by_domain
            .get(&Some(domain_id_val.clone()))
            .unwrap_or(&empty_vec);

        let mut mission_summaries: Vec<serde_json::Value> = Vec::new();

        for mission_row in domain_missions {
            let kid: String = mission_row.get("id");
            let k_name: String = mission_row.get("name");
            let k_desc: String = mission_row.get("description");
            let k_owners: String = mission_row.get("owners");
            let k_tags: Option<String> = mission_row.try_get("tags").ok().and_then(|v: Option<String>| v);

            let all_cluster_tasks = tasks_by_mission.get(&kid).unwrap_or(&empty_vec);

            let cluster_match = if needle.is_empty() {
                true
            } else {
                matches_query(
                    &needle,
                    &[
                        Some(k_name.as_str()),
                        Some(k_desc.as_str()),
                        Some(k_owners.as_str()),
                        k_tags.as_deref(),
                    ],
                )
            };

            // Tasks matching the text filter
            let matching_tasks: Vec<&&sqlx::postgres::PgRow> = if needle.is_empty() {
                all_cluster_tasks.iter().collect()
            } else {
                all_cluster_tasks
                    .iter()
                    .filter(|t| {
                        let title: String = t.get("title");
                        let desc: String = t.get("description");
                        let owner: String = t.get("owner");
                        let status: String = t.get("status");
                        matches_query(
                            &needle,
                            &[
                                Some(title.as_str()),
                                Some(desc.as_str()),
                                Some(owner.as_str()),
                                Some(status.as_str()),
                            ],
                        )
                    })
                    .collect()
            };

            if !needle.is_empty() && !domain_match && !cluster_match && matching_tasks.is_empty() {
                continue;
            }

            // Display tasks: all if domain or cluster matched, else only matching
            let display_tasks: Vec<&&sqlx::postgres::PgRow> =
                if domain_match || cluster_match || needle.is_empty() {
                    all_cluster_tasks.iter().collect()
                } else {
                    matching_tasks
                };

            // Build task status counts
            let mut status_counts: HashMap<String, i64> = HashMap::new();
            for t in &display_tasks {
                let s: String = t.get("status");
                *status_counts.entry(s).or_insert(0) += 1;
            }

            // Recent tasks (up to limit_tasks_per_cluster)
            let recent_tasks: Vec<serde_json::Value> = display_tasks
                .iter()
                .take(limit_tasks_per_cluster)
                .map(|t| {
                    json!({
                        "id": t.get::<i32, _>("id"),
                        "mission_id": t.get::<String, _>("mission_id"),
                        "title": t.get::<String, _>("title"),
                        "status": t.get::<String, _>("status"),
                        "owner": t.get::<String, _>("owner"),
                        "updated_at": t.get::<chrono::NaiveDateTime, _>("updated_at"),
                    })
                })
                .collect();

            included_mission_ids.insert(kid.clone());
            mission_summaries.push(json!({
                "id": kid,
                "domain_id": mission_row.get::<Option<String>, _>("domain_id"),
                "name": k_name,
                "description": k_desc,
                "status": mission_row.get::<String, _>("status"),
                "owners": k_owners,
                "tags": k_tags,
                "updated_at": mission_row.get::<chrono::NaiveDateTime, _>("updated_at"),
                "task_count": display_tasks.len(),
                "task_status_counts": status_counts,
                "recent_tasks": recent_tasks,
            }));
        }

        if !needle.is_empty() && !domain_match && mission_summaries.is_empty() {
            continue;
        }

        let total_tasks: usize = mission_summaries
            .iter()
            .map(|k| k["task_count"].as_u64().unwrap_or(0) as usize)
            .sum();

        domain_summaries.push(json!({
            "id": domain_id_val,
            "name": domain_name,
            "description": domain_desc,
            "status": domain_row.get::<String, _>("status"),
            "visibility": domain_row.get::<String, _>("visibility"),
            "owners": domain_owners,
            "tags": domain_tags,
            "updated_at": domain_row.get::<chrono::NaiveDateTime, _>("updated_at"),
            "mission_count": mission_summaries.len(),
            "task_count": total_tasks,
            "missions": mission_summaries,
        }));
    }

    // --- unassigned missions ---
    let mut unassigned_summaries: Vec<serde_json::Value> = Vec::new();
    let empty_vec: Vec<&sqlx::postgres::PgRow> = vec![];
    let unassigned = missions_by_domain.get(&None).unwrap_or(&empty_vec);

    for mission_row in unassigned {
        let kid: String = mission_row.get("id");
        let k_name: String = mission_row.get("name");
        let k_desc: String = mission_row.get("description");
        let k_owners: String = mission_row.get("owners");
        let k_tags: Option<String> = mission_row.try_get("tags").ok().and_then(|v: Option<String>| v);

        let all_cluster_tasks = tasks_by_mission.get(&kid).unwrap_or(&empty_vec);

        let cluster_match = if needle.is_empty() {
            true
        } else {
            matches_query(
                &needle,
                &[
                    Some(k_name.as_str()),
                    Some(k_desc.as_str()),
                    Some(k_owners.as_str()),
                    k_tags.as_deref(),
                ],
            )
        };

        let matching_tasks: Vec<&&sqlx::postgres::PgRow> = if needle.is_empty() {
            all_cluster_tasks.iter().collect()
        } else {
            all_cluster_tasks
                .iter()
                .filter(|t| {
                    let title: String = t.get("title");
                    let desc: String = t.get("description");
                    let owner: String = t.get("owner");
                    let status: String = t.get("status");
                    matches_query(
                        &needle,
                        &[
                            Some(title.as_str()),
                            Some(desc.as_str()),
                            Some(owner.as_str()),
                            Some(status.as_str()),
                        ],
                    )
                })
                .collect()
        };

        if !needle.is_empty() && !cluster_match && matching_tasks.is_empty() {
            continue;
        }

        let display_tasks: Vec<&&sqlx::postgres::PgRow> =
            if cluster_match || needle.is_empty() {
                all_cluster_tasks.iter().collect()
            } else {
                matching_tasks
            };

        let mut status_counts: HashMap<String, i64> = HashMap::new();
        for t in &display_tasks {
            let s: String = t.get("status");
            *status_counts.entry(s).or_insert(0) += 1;
        }

        let recent_tasks: Vec<serde_json::Value> = display_tasks
            .iter()
            .take(limit_tasks_per_cluster)
            .map(|t| {
                json!({
                    "id": t.get::<i32, _>("id"),
                    "mission_id": t.get::<String, _>("mission_id"),
                    "title": t.get::<String, _>("title"),
                    "status": t.get::<String, _>("status"),
                    "owner": t.get::<String, _>("owner"),
                    "updated_at": t.get::<chrono::NaiveDateTime, _>("updated_at"),
                })
            })
            .collect();

        included_mission_ids.insert(kid.clone());
        unassigned_summaries.push(json!({
            "id": kid,
            "domain_id": serde_json::Value::Null,
            "name": k_name,
            "description": k_desc,
            "status": mission_row.get::<String, _>("status"),
            "owners": k_owners,
            "tags": k_tags,
            "updated_at": mission_row.get::<chrono::NaiveDateTime, _>("updated_at"),
            "task_count": display_tasks.len(),
            "task_status_counts": status_counts,
            "recent_tasks": recent_tasks,
        }));
    }

    let total_tasks: usize = domain_summaries
        .iter()
        .map(|m| m["task_count"].as_u64().unwrap_or(0) as usize)
        .sum::<usize>()
        + unassigned_summaries
            .iter()
            .map(|k| k["task_count"].as_u64().unwrap_or(0) as usize)
            .sum::<usize>();

    Json(json!({
        "generated_at": Utc::now().naive_utc(),
        "domain_count": domain_summaries.len(),
        "mission_count": included_mission_ids.len(),
        "task_count": total_tasks,
        "domains": domain_summaries,
        "unassigned_missions": unassigned_summaries,
    }))
    .into_response()
}

// --- /explorer/node/{node_type}/{node_id} ---

async fn explorer_node(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((node_type, node_id)): Path<(String, String)>,
    Query(params): Query<NodeQuery>,
) -> impl IntoResponse {
    let limit_tasks = params.limit_tasks.unwrap_or(50).min(200).max(1);

    match node_type.as_str() {
        "domain" => {
            let domain_row = match sqlx::query("SELECT * FROM domain WHERE id=$1")
                .bind(&node_id)
                .fetch_optional(&state.db)
                .await
            {
                Ok(Some(r)) => r,
                Ok(None) => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({"detail": "Domain not found"})),
                    )
                        .into_response()
                }
                Err(e) => {
                    tracing::error!("explorer_node domain fetch: {e}");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            };

            if !can_read_domain(&state.db, &principal, &node_id).await {
                return StatusCode::FORBIDDEN.into_response();
            }

            let mission_rows = match sqlx::query(
                "SELECT * FROM mission WHERE domain_id=$1 ORDER BY updated_at DESC",
            )
            .bind(&node_id)
            .fetch_all(&state.db)
            .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::error!("explorer_node domain missions: {e}");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            };

            let mission_ids: Vec<String> =
                mission_rows.iter().map(|r| r.get::<String, _>("id")).collect();

            let task_rows: Vec<sqlx::postgres::PgRow> = if mission_ids.is_empty() {
                vec![]
            } else {
                let placeholders: String = mission_ids
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("${}", i + 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                let query_str = format!(
                    "SELECT * FROM task WHERE mission_id IN ({}) ORDER BY updated_at DESC LIMIT ${}",
                    placeholders,
                    mission_ids.len() + 1
                );
                let mut q = sqlx::query(&query_str);
                for kid in &mission_ids {
                    q = q.bind(kid);
                }
                q = q.bind(limit_tasks);
                match q.fetch_all(&state.db).await {
                    Ok(rows) => rows,
                    Err(e) => {
                        tracing::error!("explorer_node domain tasks: {e}");
                        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                }
            };

            Json(json!({
                "node_type": node_type,
                "node_id": node_id,
                "domain": row_to_domain(&domain_row),
                "missions": mission_rows.iter().map(row_to_mission).collect::<Vec<_>>(),
                "tasks": task_rows.iter().map(row_to_task).collect::<Vec<_>>(),
            }))
            .into_response()
        }

        "mission" => {
            let mission_row = match sqlx::query("SELECT * FROM mission WHERE id=$1")
                .bind(&node_id)
                .fetch_optional(&state.db)
                .await
            {
                Ok(Some(r)) => r,
                Ok(None) => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({"detail": "Mission not found"})),
                    )
                        .into_response()
                }
                Err(e) => {
                    tracing::error!("explorer_node mission fetch: {e}");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            };

            let domain_id: Option<String> = mission_row
                .try_get("domain_id")
                .ok()
                .and_then(|v: Option<String>| v);

            match &domain_id {
                Some(mid) => {
                    if !can_read_domain(&state.db, &principal, mid).await {
                        return StatusCode::FORBIDDEN.into_response();
                    }
                }
                None => {
                    if !principal.is_admin {
                        return (
                            StatusCode::FORBIDDEN,
                            Json(json!({"detail": "Forbidden: domain viewer, contributor, or owner required"})),
                        )
                            .into_response();
                    }
                }
            }

            let domain_val: serde_json::Value = match &domain_id {
                Some(mid) => {
                    match sqlx::query("SELECT * FROM domain WHERE id=$1")
                        .bind(mid)
                        .fetch_optional(&state.db)
                        .await
                    {
                        Ok(Some(r)) => row_to_domain(&r),
                        Ok(None) => serde_json::Value::Null,
                        Err(e) => {
                            tracing::error!("explorer_node mission domain fetch: {e}");
                            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                        }
                    }
                }
                None => serde_json::Value::Null,
            };

            let task_rows = match sqlx::query(
                "SELECT * FROM task WHERE mission_id=$1 ORDER BY updated_at DESC LIMIT $2",
            )
            .bind(&node_id)
            .bind(limit_tasks)
            .fetch_all(&state.db)
            .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::error!("explorer_node mission tasks: {e}");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            };

            Json(json!({
                "node_type": node_type,
                "node_id": node_id,
                "domain": domain_val,
                "mission": row_to_mission(&mission_row),
                "tasks": task_rows.iter().map(row_to_task).collect::<Vec<_>>(),
            }))
            .into_response()
        }

        "task" => {
            // Try public_id first, then numeric id
            let task_row_opt = match sqlx::query("SELECT * FROM task WHERE public_id=$1 LIMIT 1")
                .bind(&node_id)
                .fetch_optional(&state.db)
                .await
            {
                Ok(Some(r)) => Some(r),
                Ok(None) => {
                    // Try numeric id
                    if let Ok(numeric_id) = node_id.parse::<i32>() {
                        match sqlx::query("SELECT * FROM task WHERE id=$1")
                            .bind(numeric_id)
                            .fetch_optional(&state.db)
                            .await
                        {
                            Ok(r) => r,
                            Err(e) => {
                                tracing::error!("explorer_node task fetch by id: {e}");
                                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                            }
                        }
                    } else {
                        None
                    }
                }
                Err(e) => {
                    tracing::error!("explorer_node task fetch: {e}");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            };

            let task_row = match task_row_opt {
                Some(r) => r,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({"detail": "Task not found"})),
                    )
                        .into_response()
                }
            };

            let mission_id: String = task_row.get("mission_id");
            let mission_row = match sqlx::query("SELECT * FROM mission WHERE id=$1")
                .bind(&mission_id)
                .fetch_optional(&state.db)
                .await
            {
                Ok(Some(r)) => r,
                Ok(None) => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({"detail": "Mission not found"})),
                    )
                        .into_response()
                }
                Err(e) => {
                    tracing::error!("explorer_node task mission fetch: {e}");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            };

            let domain_id: Option<String> = mission_row
                .try_get("domain_id")
                .ok()
                .and_then(|v: Option<String>| v);

            match &domain_id {
                Some(mid) => {
                    if !can_read_domain(&state.db, &principal, mid).await {
                        return StatusCode::FORBIDDEN.into_response();
                    }
                }
                None => {
                    if !principal.is_admin {
                        return (
                            StatusCode::FORBIDDEN,
                            Json(json!({"detail": "Forbidden: domain viewer, contributor, or owner required"})),
                        )
                            .into_response();
                    }
                }
            }

            let domain_val: serde_json::Value = match &domain_id {
                Some(mid) => {
                    match sqlx::query("SELECT * FROM domain WHERE id=$1")
                        .bind(mid)
                        .fetch_optional(&state.db)
                        .await
                    {
                        Ok(Some(r)) => row_to_domain(&r),
                        Ok(None) => serde_json::Value::Null,
                        Err(e) => {
                            tracing::error!("explorer_node task domain fetch: {e}");
                            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                        }
                    }
                }
                None => serde_json::Value::Null,
            };

            Json(json!({
                "node_type": node_type,
                "node_id": node_id,
                "domain": domain_val,
                "mission": row_to_mission(&mission_row),
                "task": row_to_task(&task_row),
            }))
            .into_response()
        }

        _ => (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "node_type must be one of: domain, mission, task"})),
        )
            .into_response(),
    }
}
