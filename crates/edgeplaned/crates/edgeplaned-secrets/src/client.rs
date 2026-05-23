use once_cell::sync::Lazy;
use std::time::Duration;

use crate::config::InfisicalConfig;
use crate::error::{Result, SecretsError};
use crate::token_cache::TokenCache;

static GLOBAL_TOKEN_CACHE: Lazy<TokenCache> = Lazy::new(TokenCache::new);

/// Async HTTP client for the Infisical API.
/// Supports service token (legacy Bearer) and Universal Auth (client-credentials OAuth2).
pub struct InfisicalClient {
    site_url: String,
    auth: Auth,
    http: reqwest::Client,
}

enum Auth {
    ServiceToken(String),
    UniversalAuth { client_id: String, client_secret: String },
}

impl InfisicalClient {
    pub fn new(cfg: &InfisicalConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| SecretsError::Http(format!("failed to build reqwest client: {e}")))?;

        let auth = if let Some(token) = &cfg.service_token {
            if !token.trim().is_empty() {
                Auth::ServiceToken(token.clone())
            } else {
                return Err(SecretsError::Config("service_token is empty".into()));
            }
        } else if let (Some(id), Some(secret)) = (&cfg.client_id, &cfg.client_secret) {
            if id.trim().is_empty() || secret.trim().is_empty() {
                return Err(SecretsError::Config("client_id or client_secret is empty".into()));
            }
            Auth::UniversalAuth {
                client_id: id.clone(),
                client_secret: secret.clone(),
            }
        } else {
            return Err(SecretsError::Config(
                "no auth credentials configured: set service_token or client_id+client_secret".into(),
            ));
        };

        Ok(Self {
            site_url: cfg.site_url.trim_end_matches('/').to_string(),
            auth,
            http,
        })
    }

    /// Resolve a bearer token, exchanging UA credentials if needed.
    /// Uses the global in-process cache to avoid redundant token requests.
    async fn bearer_token(&self) -> Result<String> {
        match &self.auth {
            Auth::ServiceToken(t) => Ok(t.clone()),
            Auth::UniversalAuth { client_id, client_secret } => {
                if let Some(cached) = GLOBAL_TOKEN_CACHE.get(&self.site_url, client_id) {
                    return Ok(cached);
                }
                self.exchange_ua_token(client_id, client_secret).await
            }
        }
    }

    /// POST /api/v1/auth/universal-auth/login to exchange client credentials for an access token.
    async fn exchange_ua_token(&self, client_id: &str, client_secret: &str) -> Result<String> {
        let url = format!("{}/api/v1/auth/universal-auth/login", self.site_url);
        let body = serde_json::json!({
            "clientId": client_id,
            "clientSecret": client_secret,
        });

        let resp = self.http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SecretsError::Http(format!("UA login request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(SecretsError::Auth(format!("UA login {status}: {text}")));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SecretsError::Json(e.to_string()))?;

        let token = body["accessToken"]
            .as_str()
            .ok_or_else(|| SecretsError::Auth("UA login response missing accessToken".into()))?
            .to_string();

        let ttl = body["expiresIn"].as_u64().unwrap_or(7200);
        GLOBAL_TOKEN_CACHE.store(&self.site_url, client_id, token.clone(), ttl);

        tracing::debug!(client_id, ttl, "UA token acquired and cached");
        Ok(token)
    }

    /// Fetch the value of a single secret by name.
    pub async fn fetch_secret(
        &self,
        secret_name: &str,
        project_id: &str,
        environment: &str,
        path: &str,
    ) -> Result<String> {
        let token = self.bearer_token().await?;
        let url = format!(
            "{}/api/v3/secrets/raw/{}?workspaceId={}&environment={}&secretPath={}",
            self.site_url,
            secret_name,
            project_id,
            environment,
            urlencoding::encode(path),
        );
        let resp = self.http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| SecretsError::Http(format!("Infisical fetch_secret: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SecretsError::Http(format!("Infisical {status}: {body}")));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SecretsError::Json(e.to_string()))?;

        body["secret"]["secretValue"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| SecretsError::SecretNotFound(secret_name.to_string()))
    }

    /// List secret names at the given path (values are not fetched).
    pub async fn list_secrets(
        &self,
        project_id: &str,
        environment: &str,
        path: &str,
    ) -> Result<Vec<String>> {
        let token = self.bearer_token().await?;
        let url = format!(
            "{}/api/v3/secrets/raw?workspaceId={}&environment={}&secretPath={}&recursive=false",
            self.site_url,
            project_id,
            environment,
            urlencoding::encode(path),
        );
        let resp = self.http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| SecretsError::Http(format!("Infisical list_secrets: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SecretsError::Http(format!("Infisical {status}: {body}")));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SecretsError::Json(e.to_string()))?;

        Ok(body["secrets"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s["secretKey"].as_str().map(|n| n.to_string()))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// List subfolder names at the given path.
    ///
    /// Uses API v1 (`/api/v1/folders`) rather than v3 because the Infisical v3
    /// secrets endpoint does not expose folder listings.
    pub async fn list_folders(
        &self,
        project_id: &str,
        environment: &str,
        path: &str,
    ) -> Result<Vec<String>> {
        let token = self.bearer_token().await?;
        let url = format!(
            "{}/api/v1/folders?workspaceId={}&environment={}&path={}",
            self.site_url,
            project_id,
            environment,
            urlencoding::encode(path),
        );
        let resp = self.http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| SecretsError::Http(format!("Infisical list_folders: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SecretsError::Http(format!("Infisical {status}: {body}")));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SecretsError::Json(e.to_string()))?;

        Ok(body["folders"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|f| f["name"].as_str().map(|n| n.to_string()))
                    .collect()
            })
            .unwrap_or_default())
    }
}
