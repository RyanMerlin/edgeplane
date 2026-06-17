use crate::config::{load_server_list, EdgeplaneConfig};
use anyhow::{Context, Result};
use reqwest::{Client, Method, RequestBuilder};
use serde_json::Value;
use std::time::Duration;
use url::Url;

/// HTTP client that keeps auth headers, retries, and streaming helpers in one place.
#[derive(Clone)]
pub struct EdgeplaneClient {
    http: Client,
    base_url: Url,
    token: Option<String>,
    agent_id: Option<String>,
    runtime_session_id: Option<String>,
    profile_name: Option<String>,
}

impl EdgeplaneClient {
    pub fn new(config: &EdgeplaneConfig) -> Result<Self> {
        let mut builder = Client::builder().timeout(config.timeout);
        if config.allow_insecure {
            builder = builder.danger_accept_invalid_certs(true);
        }

        Ok(Self {
            http: builder.build()?,
            base_url: config.base_url.clone(),
            token: config.token.clone(),
            agent_id: config.agent_context.agent_id.clone(),
            runtime_session_id: config.agent_context.runtime_session_id.clone(),
            profile_name: config.agent_context.profile_name.clone(),
        })
    }

    /// Build a minimal client from a raw base URL + token, used during `edgeplane auth login`
    /// before a full EdgeplaneConfig is available.
    pub fn new_with_token(base_url: &str, token: &str) -> Result<Self> {
        let http = Client::builder().timeout(Duration::from_secs(15)).build()?;
        Ok(Self {
            http,
            base_url: Url::parse(base_url).context("invalid base URL")?,
            token: if token.is_empty() {
                None
            } else {
                Some(token.to_string())
            },
            agent_id: None,
            runtime_session_id: None,
            profile_name: None,
        })
    }

    fn apply_auth(&self, builder: RequestBuilder) -> RequestBuilder {
        let mut request = if let Some(token) = &self.token {
            builder.bearer_auth(token)
        } else {
            builder
        };
        if let Some(agent_id) = &self.agent_id
            && !agent_id.trim().is_empty() {
                request = request.header("x-edgeplane-agent-id", agent_id);
            }
        if let Some(runtime_session_id) = &self.runtime_session_id
            && !runtime_session_id.trim().is_empty() {
                request = request.header("x-edgeplane-runtime-session-id", runtime_session_id);
                request = request.header("x-edgeplane-instance-id", runtime_session_id);
            }
        if let Some(profile_name) = &self.profile_name
            && !profile_name.trim().is_empty() {
                request = request.header("x-edgeplane-agent-profile", profile_name);
            }
        request
    }

    pub fn request_builder(&self, method: Method, path: &str) -> Result<RequestBuilder> {
        let api_prefix = std::env::var("EP_API_PREFIX").unwrap_or_else(|_| "/api".to_string());
        let prefixed = if path.starts_with('/') {
            format!("{}{}", api_prefix.trim_end_matches('/'), path)
        } else {
            path.to_string()
        };
        let url = self
            .base_url
            .join(&prefixed)
            .with_context(|| format!("invalid endpoint path: {path}"))?;
        Ok(self.apply_auth(self.http.request(method, url)))
    }

    /// Check a response's status and surface the server's error body on non-2xx,
    /// matching the error shape used by [`MultiServerClient`] lower in this file.
    async fn check_response(resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let text = resp.text().await.unwrap_or_default();
        Err(anyhow::anyhow!("HTTP {status}: {text}"))
    }

    pub async fn get_json(&self, path: &str) -> Result<Value> {
        let resp = self
            .request_builder(Method::GET, path)?
            .send()
            .await
            .context("request failed")?;
        Self::check_response(resp)
            .await?
            .json::<Value>()
            .await
            .context("unable to parse json response")
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Value> {
        let resp = self
            .request_builder(Method::POST, path)?
            .json(body)
            .send()
            .await
            .context("request failed")?;
        Self::check_response(resp)
            .await?
            .json::<Value>()
            .await
            .context("unable to parse json response")
    }

    pub async fn patch_json(&self, path: &str, body: &Value) -> Result<Value> {
        let resp = self
            .request_builder(Method::PATCH, path)?
            .json(body)
            .send()
            .await
            .context("request failed")?;
        Self::check_response(resp)
            .await?
            .json::<Value>()
            .await
            .context("unable to parse json response")
    }

    pub async fn put_json(&self, path: &str, body: &Value) -> Result<Value> {
        let resp = self
            .request_builder(Method::PUT, path)?
            .json(body)
            .send()
            .await
            .context("request failed")?;
        Self::check_response(resp)
            .await?
            .json::<Value>()
            .await
            .context("unable to parse json response")
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let resp = self
            .request_builder(Method::DELETE, path)?
            .send()
            .await
            .context("request failed")?;
        Self::check_response(resp).await?;
        Ok(())
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub fn ws_url(&self, path: &str) -> Result<url::Url> {
        let mut url = self.base_url.clone();
        match url.scheme() {
            "http" => {
                url.set_scheme("ws").ok();
            }
            "https" => {
                url.set_scheme("wss").ok();
            }
            _ => {}
        }
        let api_prefix = std::env::var("EP_API_PREFIX").unwrap_or_else(|_| "/api".to_string());
        let prefixed = if path.starts_with('/') {
            format!("{}{}", api_prefix.trim_end_matches('/'), path)
        } else {
            path.to_string()
        };
        url = url
            .join(&prefixed)
            .with_context(|| format!("invalid websocket path: {path}"))?;
        Ok(url)
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Return a clone of this client pointing at a different base URL.
    pub fn with_base_url(&self, base_url: Url) -> Self {
        Self {
            http: self.http.clone(),
            base_url,
            token: self.token.clone(),
            agent_id: self.agent_id.clone(),
            runtime_session_id: self.runtime_session_id.clone(),
            profile_name: self.profile_name.clone(),
        }
    }
}

// ── Multi-server failover client ──────────────────────────────────────────────

/// Wraps a list of edgeplane-tower endpoints and retries each in order on connection
/// failure or 5xx. Reads the server list from `~/.ep/servers` (via
/// `load_server_list()`) unless overridden with explicit URLs.
///
/// HTTP 4xx errors from the first responding server are returned immediately
/// without trying the next — they indicate a bad request, not a bad server.
#[derive(Clone)]
pub struct MultiServerClient {
    servers: Vec<Url>,
    inner: Client,
    token: Option<String>,
    agent_id: Option<String>,
    runtime_session_id: Option<String>,
    profile_name: Option<String>,
}

impl MultiServerClient {
    /// Build from a full EdgeplaneConfig, using `load_server_list()` for the URL list.
    pub fn from_config(config: &EdgeplaneConfig) -> Result<Self> {
        let servers = load_server_list()
            .iter()
            .map(|u| Url::parse(u).with_context(|| format!("invalid server URL: {u}")))
            .collect::<Result<Vec<_>>>()?;

        let mut builder = Client::builder().timeout(config.timeout);
        if config.allow_insecure {
            builder = builder.danger_accept_invalid_certs(true);
        }
        Ok(Self {
            servers,
            inner: builder.build()?,
            token: config.token.clone(),
            agent_id: config.agent_context.agent_id.clone(),
            runtime_session_id: config.agent_context.runtime_session_id.clone(),
            profile_name: config.agent_context.profile_name.clone(),
        })
    }

    /// Build with an explicit list of server URLs (useful for testing).
    pub fn new(servers: Vec<String>, timeout: Duration) -> Result<Self> {
        let parsed = servers
            .iter()
            .map(|u| Url::parse(u).with_context(|| format!("invalid server URL: {u}")))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            servers: parsed,
            inner: Client::builder()
                .timeout(timeout)
                .build()
                .context("http client init")?,
            token: None,
            agent_id: None,
            runtime_session_id: None,
            profile_name: None,
        })
    }

    fn apply_auth(&self, builder: RequestBuilder) -> RequestBuilder {
        let mut req = if let Some(token) = &self.token {
            builder.bearer_auth(token)
        } else {
            builder
        };
        if let Some(id) = &self.agent_id
            && !id.trim().is_empty() {
                req = req.header("x-edgeplane-agent-id", id);
            }
        if let Some(rsid) = &self.runtime_session_id
            && !rsid.trim().is_empty() {
                req = req
                    .header("x-edgeplane-runtime-session-id", rsid)
                    .header("x-edgeplane-instance-id", rsid);
            }
        if let Some(profile) = &self.profile_name
            && !profile.trim().is_empty() {
                req = req.header("x-edgeplane-agent-profile", profile);
            }
        req
    }

    /// Try each server in order. Returns the first successful response.
    /// Connection errors and 5xx advance to the next server.
    /// 4xx from any server is returned immediately.
    pub async fn get_json(&self, path: &str) -> Result<Value> {
        self.try_servers(Method::GET, path, None).await
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Value> {
        self.try_servers(Method::POST, path, Some(body)).await
    }

    pub async fn patch_json(&self, path: &str, body: &Value) -> Result<Value> {
        self.try_servers(Method::PATCH, path, Some(body)).await
    }

    pub async fn put_json(&self, path: &str, body: &Value) -> Result<Value> {
        self.try_servers(Method::PUT, path, Some(body)).await
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        self.try_servers(Method::DELETE, path, None).await?;
        Ok(())
    }

    async fn try_servers(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value> {
        let api_prefix = std::env::var("EP_API_PREFIX").unwrap_or_else(|_| "/api".to_string());
        let prefixed = if path.starts_with('/') {
            format!("{}{}", api_prefix.trim_end_matches('/'), path)
        } else {
            path.to_string()
        };
        let mut last_err = anyhow::anyhow!("no edgeplane-tower nodes configured");
        for base in &self.servers {
            let url = base
                .join(&prefixed)
                .with_context(|| format!("invalid path: {path}"))?;
            let mut builder = self.apply_auth(self.inner.request(method.clone(), url));
            if let Some(b) = body {
                builder = builder.json(b);
            }
            match builder.send().await {
                Err(e) => {
                    tracing::debug!(server = %base, error = %e, "edgeplane-tower unreachable, trying next");
                    last_err = e.into();
                }
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_client_error() {
                        // 4xx — bad request, not a bad server; return immediately
                        let text = resp.text().await.unwrap_or_default();
                        return Err(anyhow::anyhow!("HTTP {status}: {text}"));
                    }
                    if status.is_server_error() {
                        tracing::debug!(server = %base, %status, "edgeplane-tower 5xx, trying next");
                        last_err = anyhow::anyhow!("HTTP {status} from {base}");
                        continue;
                    }
                    return resp
                        .json::<Value>()
                        .await
                        .context("unable to parse json response");
                }
            }
        }
        Err(last_err)
    }

    pub fn server_count(&self) -> usize {
        self.servers.len()
    }
}
