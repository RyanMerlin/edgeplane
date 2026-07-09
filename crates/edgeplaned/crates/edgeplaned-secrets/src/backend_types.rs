use std::fmt;

use zeroize::Zeroize;

/// A resolved secret. Zeroized on drop; never rendered in logs.
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(v: String) -> Self {
        SecretValue(v)
    }

    /// Deliberately-named accessor — every call site is an audit point.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretValue([REDACTED])")
    }
}

impl fmt::Display for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED]")
    }
}

/// What a backend can do beyond `resolve`. Default = nothing extra (Guard 1).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BackendCapabilities {
    pub scoped_identity: bool,
    pub leasing: bool,
    pub revoke: bool,
    pub writeback: bool,
}

/// Who is asking and why — carried into every resolve for audit + identity selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolveCtx {
    pub agent_id: Option<String>,
    pub purpose: String,
}

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("secret not found: {0}")]
    NotFound(String),
    #[error("backend unavailable: {0}")]
    Unavailable(String),
    #[error("capability not supported by backend: {0}")]
    Unsupported(&'static str),
    #[error("backend error: {0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_value_debug_is_redacted() {
        let v = SecretValue::new("super-secret-token".to_string());
        assert_eq!(format!("{v:?}"), "SecretValue([REDACTED])");
        assert_eq!(format!("{v}"), "[REDACTED]");
        assert_eq!(v.expose(), "super-secret-token");
    }

    #[test]
    fn capabilities_default_all_false() {
        let c = BackendCapabilities::default();
        assert!(!c.scoped_identity && !c.leasing && !c.revoke && !c.writeback);
    }
}
