use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// Per-dispatch credential session registry.
///
/// The dispatcher creates a session (pre-resolved credential values keyed by
/// inject_as name) and gives the agent a session ID via `MC_SECRETS_SESSION`.
/// The agent requests individual values at runtime through the secrets gateway
/// socket. Sessions are removed when the subprocess exits.
pub struct SessionStore {
    inner: Mutex<HashMap<String, HashMap<String, String>>>,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a set of resolved credentials. Returns the new session ID.
    pub fn create(&self, creds: HashMap<String, String>) -> String {
        let id = Uuid::new_v4().to_string();
        self.inner.lock().unwrap().insert(id.clone(), creds);
        id
    }

    /// Look up a single credential value by session ID and env-var name.
    pub fn get(&self, session_id: &str, name: &str) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .get(session_id)
            .and_then(|m| m.get(name))
            .cloned()
    }

    /// Remove a session once the owning subprocess has exited.
    pub fn remove(&self, session_id: &str) {
        self.inner.lock().unwrap().remove(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_get_remove_roundtrip() {
        let store = SessionStore::new();
        let mut creds = HashMap::new();
        creds.insert("API_KEY".to_string(), "secret-value".to_string());
        let id = store.create(creds);

        assert_eq!(store.get(&id, "API_KEY").as_deref(), Some("secret-value"));
        assert!(store.get(&id, "MISSING").is_none());

        store.remove(&id);
        assert!(store.get(&id, "API_KEY").is_none());
    }

    #[test]
    fn sessions_are_isolated() {
        let store = SessionStore::new();
        let mut a = HashMap::new(); a.insert("K".to_string(), "v_a".to_string());
        let mut b = HashMap::new(); b.insert("K".to_string(), "v_b".to_string());
        let id_a = store.create(a);
        let id_b = store.create(b);

        assert_eq!(store.get(&id_a, "K").as_deref(), Some("v_a"));
        assert_eq!(store.get(&id_b, "K").as_deref(), Some("v_b"));
    }
}
