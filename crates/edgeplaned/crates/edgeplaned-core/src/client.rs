use anyhow::Result;
use reqwest::{Client, Response};
use serde::{de::DeserializeOwned, Serialize};
use std::sync::{Arc, RwLock};

/// Thin HTTP client with bearer auth for the Edgeplane backend.
///
/// `api_prefix` is prepended to every path passed into `get`/`post`/etc. The
/// daemon sets it to `/api` (see `edgeplaned-bin/src/config.rs`, env-overridable
/// via `EP_API_PREFIX`) to match the tower's `.nest("/api", ...)` mount.
///
/// Within that `/api` root, MeshTask dispatch operations (claim, heartbeat,
/// progress, complete, fail, and mission task listing) live under a further
/// `/work` segment — see `edgeplane-tower/src/routes/work.rs`'s `router()`.
/// Callers of those operations must include the `/work` segment explicitly in
/// the path passed to `get`/`post`/etc.; it is not part of `api_prefix`.
///
/// The bearer token is held in an `Arc<RwLock<String>>` so that all clones
/// of a `BackendClient` share the same live credential.  Calling `set_token`
/// on any handle updates every consumer (heartbeat loop, WS reconnect, task
/// worker) atomically.
#[derive(Clone)]
pub struct BackendClient {
    pub base_url: String,
    token: Arc<RwLock<String>>,
    pub api_prefix: String,
    inner: Client,
}

impl BackendClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        BackendClient {
            base_url: base_url.into(),
            token: Arc::new(RwLock::new(token.into())),
            api_prefix: String::new(),
            inner: Client::new(),
        }
    }

    /// Override the API prefix that's prepended to every request path. Pass
    /// an empty string for the default "no prefix" controlplane.
    pub fn with_api_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.api_prefix = prefix.into();
        self
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}{}{}",
            self.base_url.trim_end_matches('/'),
            self.api_prefix,
            path,
        )
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token.read().unwrap())
    }

    /// Return a snapshot of the current bearer token.
    ///
    /// Acquires the read lock momentarily; safe to call from any context
    /// including inside an async task (the lock is held only for the clone,
    /// never across an `.await`).
    pub fn current_token(&self) -> String {
        self.token.read().unwrap().clone()
    }

    /// Replace the bearer token used by this client and all its clones.
    ///
    /// All subsequent requests issued by any clone will use the new value.
    /// The write lock is held only for the assignment — never across `.await`.
    pub fn set_token(&self, new: impl Into<String>) {
        *self.token.write().unwrap() = new.into();
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self
            .inner
            .get(self.url(path))
            .header("Authorization", self.auth_header())
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let resp = self
            .inner
            .post(self.url(path))
            .header("Authorization", self.auth_header())
            .json(body)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn post_empty<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self
            .inner
            .post(self.url(path))
            .header("Authorization", self.auth_header())
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn raw_post<B: Serialize>(&self, path: &str, body: &B) -> Result<Response> {
        Ok(self
            .inner
            .post(self.url(path))
            .header("Authorization", self.auth_header())
            .json(body)
            .send()
            .await?
            .error_for_status()?)
    }

    /// Like `raw_post` but does not call `error_for_status()` — the caller
    /// inspects the status code directly (e.g. to detect 409 lease mismatch).
    pub async fn raw_post_no_throw<B: Serialize>(&self, path: &str, body: &B) -> Result<Response> {
        Ok(self
            .inner
            .post(self.url(path))
            .header("Authorization", self.auth_header())
            .json(body)
            .send()
            .await?)
    }

    pub async fn patch<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let resp = self
            .inner
            .patch(self.url(path))
            .header("Authorization", self.auth_header())
            .json(body)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// DELETE request; expects a 200/204 response. Returns the raw Response so
    /// the caller can check the status code if needed.
    pub async fn delete(&self, path: &str) -> Result<Response> {
        Ok(self
            .inner
            .delete(self.url(path))
            .header("Authorization", self.auth_header())
            .send()
            .await?
            .error_for_status()?)
    }

    /// Fetch the domain roster — concise agent list for prompt injection.
    pub async fn get_domain_roster(&self, domain_id: &str) -> Result<Vec<serde_json::Value>> {
        self.get(&format!("/domains/{domain_id}/roster")).await
    }

    /// Fetch a single agent's full detail (includes profile/machine/runtime).
    pub async fn get_agent(&self, agent_id: &str) -> Result<serde_json::Value> {
        self.get(&format!("/agents/{agent_id}")).await
    }

    /// Update an agent's profile.
    pub async fn update_agent_profile(
        &self,
        agent_id: &str,
        profile: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.patch(&format!("/agents/{agent_id}/profile"), profile).await
    }

    /// Mint a fresh per-agent JWT for `agent_id` via the full-trust-gated
    /// `POST /work/agents/{agent_id}/token` endpoint. The daemon authenticates
    /// with its own node credential (full-trust → authorized) and injects the
    /// returned token as that agent's `EP_AGENT_TOKEN`, so the agent acts as a
    /// domain-scoped principal rather than the shared daemon.
    ///
    /// The path mirrors the enroll call's convention (bare `/work/...`; the
    /// configured `api_prefix` is prepended by `url()`). The endpoint takes no
    /// request body and responds with `{"agent_token": "...", "expires_in": ...}`.
    pub async fn mint_agent_token(&self, agent_id: &str) -> Result<String> {
        #[derive(serde::Deserialize)]
        struct MintTokenResponse {
            agent_token: String,
        }
        let resp: MintTokenResponse = self
            .post_empty(&format!("/work/agents/{agent_id}/token"))
            .await?;
        Ok(resp.agent_token)
    }

    /// Call `POST /runtime/nodes/{node_id}/rotate-token` (no body) and return
    /// the freshly-issued node JWT.
    ///
    /// The tower revokes the current active JTI and issues a new one with a
    /// 24-h TTL.  The daemon's rotation loop calls this, then persists the new
    /// token to `node.json` before calling `set_token` so a crash between
    /// persist and live-swap doesn't leave the stored credential stale.
    pub async fn rotate_node_token(&self, node_id: &str) -> Result<String> {
        #[derive(serde::Deserialize)]
        struct RotateResponse {
            node_jwt: String,
        }
        let resp: RotateResponse = self
            .post_empty(&format!("/runtime/nodes/{node_id}/rotate-token"))
            .await?;
        Ok(resp.node_jwt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_token_is_observed_by_clone() {
        let client = BackendClient::new("http://localhost:8008", "initial-token");
        let cloned = client.clone();

        // Mutation on the original is seen by the clone.
        client.set_token("rotated-token");
        assert_eq!(cloned.current_token(), "rotated-token");
    }

    #[test]
    fn set_token_is_observed_by_original_after_clone_mutates() {
        let client = BackendClient::new("http://localhost:8008", "v1");
        let cloned = client.clone();

        // Mutation on the clone is seen by the original.
        cloned.set_token("v2");
        assert_eq!(client.current_token(), "v2");
    }

    #[test]
    fn auth_header_reflects_current_token() {
        let client = BackendClient::new("http://localhost:8008", "tok-a");
        assert_eq!(client.auth_header(), "Bearer tok-a");
        client.set_token("tok-b");
        assert_eq!(client.auth_header(), "Bearer tok-b");
    }

    #[test]
    fn multiple_clones_share_one_arc() {
        let c1 = BackendClient::new("http://localhost:8008", "start");
        let c2 = c1.clone();
        let c3 = c2.clone();
        c3.set_token("end");
        assert_eq!(c1.current_token(), "end");
        assert_eq!(c2.current_token(), "end");
    }
}
