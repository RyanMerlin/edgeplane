use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A cached Universal Auth token.
struct CachedToken {
    token: String,
    expires_at: Instant,
}

impl CachedToken {
    fn is_valid(&self) -> bool {
        self.expires_at > Instant::now()
    }
}

/// In-process cache for Universal Auth access tokens.
///
/// Keyed by `(site_url, client_id)` so each UA identity gets its own slot.
/// Tokens are refreshed when within 60 s of expiry.
#[derive(Default)]
pub struct TokenCache {
    inner: Mutex<HashMap<(String, String), CachedToken>>,
}

impl TokenCache {
    pub fn new() -> Self { Self::default() }

    /// Return a cached token for the given identity, if still valid.
    pub fn get(&self, site_url: &str, client_id: &str) -> Option<String> {
        let guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let key = (site_url.to_string(), client_id.to_string());
        guard.get(&key).filter(|t| t.is_valid()).map(|t| t.token.clone())
    }

    /// Store a token with a TTL (in seconds).  Stores `ttl - 60` to refresh
    /// 1 minute before actual expiry.
    pub fn store(&self, site_url: &str, client_id: &str, token: String, ttl_secs: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let key = (site_url.to_string(), client_id.to_string());
        let expires_at = Instant::now()
            + Duration::from_secs(ttl_secs.saturating_sub(60).max(30));
        guard.insert(key, CachedToken { token, expires_at });
    }

    /// Evict all entries — useful in tests.
    pub fn clear(&self) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_miss_before_store() {
        let cache = TokenCache::new();
        assert!(cache.get("https://app.infisical.com", "id1").is_none());
    }

    #[test]
    fn cache_hit_after_store() {
        let cache = TokenCache::new();
        cache.store("https://app.infisical.com", "id1", "tok123".into(), 3600);
        assert_eq!(cache.get("https://app.infisical.com", "id1").unwrap(), "tok123");
    }

    #[test]
    fn different_identities_independent() {
        let cache = TokenCache::new();
        cache.store("https://a.com", "id1", "tok-a".into(), 3600);
        cache.store("https://b.com", "id2", "tok-b".into(), 3600);
        assert_eq!(cache.get("https://a.com", "id1").unwrap(), "tok-a");
        assert_eq!(cache.get("https://b.com", "id2").unwrap(), "tok-b");
        assert!(cache.get("https://a.com", "id2").is_none());
    }

    #[test]
    fn short_ttl_still_stores_minimum() {
        let cache = TokenCache::new();
        // TTL of 1 sec → effective 30 s (min), token should still be valid right now
        cache.store("https://app.infisical.com", "id1", "tok_short".into(), 1);
        assert_eq!(cache.get("https://app.infisical.com", "id1").unwrap(), "tok_short");
    }
}
