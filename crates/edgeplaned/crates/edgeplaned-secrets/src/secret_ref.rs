use std::collections::BTreeMap;
use std::fmt;

use crate::error::SecretsError;

/// A parsed `secret://<scheme>/<path>[?k=v&…]` reference. Value type, not an entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRef {
    pub scheme: String,
    pub path: String,
    pub query: BTreeMap<String, String>,
}

impl SecretRef {
    pub fn parse(input: &str) -> Result<Self, SecretsError> {
        let rest = input
            .strip_prefix("secret://")
            .ok_or_else(|| SecretsError::InvalidRef(format!("missing `secret://` prefix: {input}")))?;

        let (locator, query_str) = match rest.split_once('?') {
            Some((l, q)) => (l, Some(q)),
            None => (rest, None),
        };

        let (scheme, path) = locator
            .split_once('/')
            .ok_or_else(|| SecretsError::InvalidRef(format!("missing scheme/path separator: {input}")))?;

        if scheme.is_empty() || path.is_empty() {
            return Err(SecretsError::InvalidRef(format!("empty scheme or path: {input}")));
        }

        let mut query = BTreeMap::new();
        if let Some(q) = query_str {
            for pair in q.split('&').filter(|s| !s.is_empty()) {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                query.insert(k.to_string(), v.to_string());
            }
        }

        Ok(SecretRef { scheme: scheme.to_string(), path: path.to_string(), query })
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "secret://{}/{}", self.scheme, self.path)?;
        if !self.query.is_empty() {
            let q: Vec<String> = self.query.iter().map(|(k, v)| format!("{k}={v}")).collect();
            write!(f, "?{}", q.join("&"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scheme_and_path() {
        let r = SecretRef::parse("secret://infisical/providers/openai/OPENAI_API_KEY").unwrap();
        assert_eq!(r.scheme, "infisical");
        assert_eq!(r.path, "providers/openai/OPENAI_API_KEY");
        assert!(r.query.is_empty());
    }

    #[test]
    fn parses_query_params() {
        let r = SecretRef::parse("secret://infisical/providers/x/KEY?env=prod&project=abc").unwrap();
        assert_eq!(r.query.get("env").map(String::as_str), Some("prod"));
        assert_eq!(r.query.get("project").map(String::as_str), Some("abc"));
    }

    #[test]
    fn rejects_missing_scheme() {
        assert!(SecretRef::parse("https://example.com/x").is_err());
        assert!(SecretRef::parse("providers/openai/KEY").is_err());
    }

    #[test]
    fn display_round_trips() {
        let s = "secret://infisical/providers/x/KEY?env=prod";
        assert_eq!(SecretRef::parse(s).unwrap().to_string(), s);
    }
}
