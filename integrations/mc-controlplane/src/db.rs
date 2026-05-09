use sqlx::PgPool;
use std::env;

pub async fn connect() -> anyhow::Result<PgPool> {
    let url = build_url();
    let pool = PgPool::connect(&url).await?;
    Ok(pool)
}

fn build_url() -> String {
    if let Ok(url) = env::var("DATABASE_URL") {
        return url;
    }
    let host = env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".into());
    let port = env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".into());
    let user = env::var("POSTGRES_USER").unwrap_or_else(|_| "postgres".into());
    let password = env::var("POSTGRES_PASSWORD").unwrap_or_default();
    let db = env::var("POSTGRES_DB").unwrap_or_else(|_| "missioncontrol".into());
    format!("postgres://{user}:{password}@{host}:{port}/{db}")
}
