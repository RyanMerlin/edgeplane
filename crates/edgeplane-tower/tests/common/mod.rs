//! Shared integration-test harness: migrated Postgres + seeded domain/mission +
//! token minting. Env-gated on TEST_DATABASE_URL so DB-less CI just skips.
#![allow(dead_code)]
use edgeplane_tower::auth::hash_token;
use sqlx::PgPool;
use uuid::Uuid;

pub struct Ctx {
    pub domain_id: String,
    pub other_domain_id: String,
    pub mission_id: String,
    pub owner_session_token: String,
    pub outsider_sa_token: String,
    pub member_sa_token: String,
}

/// Insert a usersession row and return the raw `mcs_` token the extractor accepts.
///
/// Fills all NOT-NULL columns in the base schema (0001):
///   subject, token_hash, token_prefix, expires_at, created_at,
///   last_used_at, user_agent, revoked
/// email is added by migration 0003 (nullable) and supplied here for completeness.
pub async fn mint_session(db: &PgPool, subject: &str, email: &str) -> String {
    let token = format!("mcs_{}", Uuid::new_v4().simple());
    let token_hash = hash_token(&token);
    // token_prefix = first 8 chars of the raw token (convention matching make_token callers)
    let token_prefix = &token[..token.len().min(8)];
    sqlx::query(
        "INSERT INTO usersession \
         (subject, token_hash, token_prefix, expires_at, created_at, last_used_at, \
          user_agent, revoked, email) \
         VALUES ($1, $2, $3, now() + interval '1 hour', now(), now(), 'test-harness', false, $4)",
    )
    .bind(subject)
    .bind(&token_hash)
    .bind(token_prefix)
    .bind(email)
    .execute(db)
    .await
    .expect("insert usersession");
    token
}

/// Insert a serviceaccount + token row and return the raw `mcs_sa_` token.
///
/// serviceaccount NOT-NULL columns: name, owner_subject, client_secret_hash,
///   client_secret_prefix (DEFAULT ''), created_at, revoked (DEFAULT false).
/// serviceaccounttoken NOT-NULL columns: service_account_id, token_hash,
///   token_prefix (DEFAULT ''), created_at, revoked (DEFAULT false).
pub async fn mint_sa(db: &PgPool, name: &str) -> String {
    let token = format!("mcs_sa_{}", Uuid::new_v4().simple());
    let token_hash = hash_token(&token);
    let token_prefix = &token[..token.len().min(10)];
    // client_secret_hash / prefix are required; use a stable placeholder — these
    // columns back a separate OAuth client-credentials flow, not the token lookup.
    let sa_id: i32 = sqlx::query_scalar(
        "INSERT INTO serviceaccount \
         (name, owner_subject, client_secret_hash, client_secret_prefix, created_at) \
         VALUES ($1, 'harness', 'placeholder', '', now()) \
         RETURNING id",
    )
    .bind(name)
    .fetch_one(db)
    .await
    .expect("insert serviceaccount");
    sqlx::query(
        "INSERT INTO serviceaccounttoken \
         (service_account_id, token_hash, token_prefix, created_at) \
         VALUES ($1, $2, $3, now())",
    )
    .bind(sa_id)
    .bind(&token_hash)
    .bind(token_prefix)
    .execute(db)
    .await
    .expect("insert serviceaccounttoken");
    token
}

/// Insert a meshtask row with `status='running'` and a specific `claimed_by_agent_id`.
///
/// meshtask NOT-NULL columns (0001_initial_schema.sql):
///   id, mission_id, domain_id, title, claim_policy, status,
///   priority, version_counter, created_by_subject, created_at, updated_at.
/// Returns the inserted task id.
pub async fn seed_claimed_task(db: &PgPool, mission_id: &str, domain_id: &str, claimed_by: &str) -> String {
    let task_id = format!("task-{}", Uuid::new_v4().simple());
    // Supplying empty-string defaults for all text columns that row_to_task reads
    // as non-Option &str (description, depends_on, produces, consumes,
    // required_capabilities, input_json) to avoid UnexpectedNullError panics.
    sqlx::query(
        "INSERT INTO meshtask \
         (id, mission_id, domain_id, title, description, input_json, claim_policy, \
          depends_on, produces, consumes, required_capabilities, \
          status, priority, version_counter, created_by_subject, claimed_by_agent_id, \
          created_at, updated_at) \
         VALUES ($1, $2, $3, 'test-task', '', '{}', 'any', '[]', '{}', '{}', '[]', \
                 'running', 0, 1, 'harness', $4, now(), now())",
    )
    .bind(&task_id)
    .bind(mission_id)
    .bind(domain_id)
    .bind(claimed_by)
    .execute(db)
    .await
    .expect("insert meshtask");
    task_id
}

pub async fn setup() -> Option<(PgPool, Ctx)> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let db = PgPool::connect(&url)
        .await
        .expect("connect TEST_DATABASE_URL");
    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .expect("migrate");

    let domain_id = format!("dom-{}", Uuid::new_v4().simple());
    let other_domain_id = format!("dom-{}", Uuid::new_v4().simple());
    let owner_email = format!("owner-{}@example.com", Uuid::new_v4().simple());

    // Mint tokens first so we know the SA name for the contributors field.
    let member_token = mint_sa(&db, &format!("member-{}", Uuid::new_v4().simple())).await;
    let outsider_token = mint_sa(&db, &format!("outsider-{}", Uuid::new_v4().simple())).await;
    let owner_token = mint_session(&db, &owner_email, &owner_email).await;

    // The SA subject the extractor produces is `sa:{name}` — record it as a contributor.
    let member_sa_name: String = sqlx::query_scalar(
        "SELECT sa.name FROM serviceaccount sa \
         JOIN serviceaccounttoken t ON t.service_account_id = sa.id \
         WHERE t.token_hash = $1",
    )
    .bind(hash_token(&member_token))
    .fetch_one(&db)
    .await
    .expect("fetch member SA name");

    // domain NOT-NULL columns (from 0001): id, name, description, owners,
    //   contributors, tags, visibility, status, northstar_md, northstar_version,
    //   northstar_created_by, northstar_modified_by, created_at, updated_at.
    // kind has DEFAULT 'work' NOT NULL (no need to supply explicitly).
    // The owners CHECK constraint requires a non-empty trimmed value.
    for did in [&domain_id, &other_domain_id] {
        sqlx::query(
            "INSERT INTO domain \
             (id, name, description, owners, contributors, tags, visibility, status, \
              northstar_md, northstar_version, northstar_created_by, northstar_modified_by, \
              created_at, updated_at) \
             VALUES ($1, $1, '', $2, $3, '', 'private', 'active', '', 0, '', '', now(), now())",
        )
        .bind(did)
        .bind(&owner_email)
        .bind(format!("sa:{member_sa_name}"))
        .execute(&db)
        .await
        .expect("insert domain");
    }

    // mission NOT-NULL columns (from 0001): id, name, description, owners,
    //   contributors, tags, status, workstream_md, workstream_version,
    //   workstream_created_by, workstream_modified_by, created_at, updated_at.
    // domain_id is nullable FK. owners CHECK constraint: non-empty trimmed value.
    let mission_id = format!("mis-{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO mission \
         (id, domain_id, name, description, owners, contributors, tags, status, \
          workstream_md, workstream_version, workstream_created_by, workstream_modified_by, \
          created_at, updated_at) \
         VALUES ($1, $2, 'm', '', $3, '', '', 'active', '', 0, '', '', now(), now())",
    )
    .bind(&mission_id)
    .bind(&domain_id)
    .bind(&owner_email)
    .execute(&db)
    .await
    .expect("insert mission");

    Some((
        db,
        Ctx {
            domain_id,
            other_domain_id,
            mission_id,
            owner_session_token: owner_token,
            outsider_sa_token: outsider_token,
            member_sa_token: member_token,
        },
    ))
}
