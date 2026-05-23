use std::collections::HashMap;

/// Obfuscated preview of a secret value for display purposes.
/// Returns first-2 + bullets + last-4 for values ≥12 chars,
/// last-4 only for 8-11 chars, all bullets for 1-7 chars,
/// "(unset)" for empty.
pub fn preview(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let n = chars.len();
    match n {
        0 => "(unset)".into(),
        1..=7 => "•".repeat(n),
        8..=11 => {
            let tail: String = chars[n.saturating_sub(4)..].iter().collect();
            format!("{}{}", "•".repeat(n.saturating_sub(4)), tail)
        }
        _ => {
            // n >= 12: show first 2, bullets, last 4
            let head: String = chars[..2].iter().collect();
            let tail: String = chars[n.saturating_sub(4)..].iter().collect();
            format!("{}{}{}", head, "•".repeat(n.saturating_sub(6)), tail)
        }
    }
}

/// Replaces known secret values with `[REDACTED]` in any string output.
pub struct SecretRedactor {
    secrets: Vec<String>,
}

impl SecretRedactor {
    pub fn new(resolved: HashMap<String, String>) -> Self {
        let mut secrets: Vec<String> = resolved.into_values().filter(|v| !v.is_empty()).collect();
        // Sort longest-first so longer matches take precedence over shorter prefixes.
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        SecretRedactor { secrets }
    }

    pub fn redact(&self, text: &str) -> String {
        let mut result = text.to_string();
        for secret in &self.secrets {
            result = result.replace(secret.as_str(), "[REDACTED]");
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redacts_secret_values() {
        let secrets = HashMap::from([("TOKEN".to_string(), "supersecret123".to_string())]);
        let r = SecretRedactor::new(secrets);
        assert_eq!(r.redact("token: supersecret123"), "token: [REDACTED]");
    }

    #[test]
    fn test_empty_passthrough() {
        let r = SecretRedactor::new(HashMap::new());
        assert_eq!(r.redact("no secrets here"), "no secrets here");
    }

    #[test]
    fn test_longest_match_first() {
        let secrets = HashMap::from([
            ("A".to_string(), "abc".to_string()),
            ("B".to_string(), "abcdef".to_string()),
        ]);
        let r = SecretRedactor::new(secrets);
        assert_eq!(r.redact("value: abcdef"), "value: [REDACTED]");
    }

    #[test]
    fn test_preview_empty() {
        assert_eq!(preview(""), "(unset)");
    }

    #[test]
    fn test_preview_short_5() {
        assert_eq!(preview("abcde"), "•••••");
    }

    #[test]
    fn test_preview_mid_9() {
        let v = "abcdefghi";
        let result = preview(v);
        assert!(result.starts_with("•••••"));
        assert!(result.ends_with("fghi"));
    }

    #[test]
    fn test_preview_16() {
        let v = "abcdefghijklmnop";
        let result = preview(v);
        assert!(result.starts_with("ab"));
        assert!(result.ends_with("mnop"));
    }

    #[test]
    fn test_preview_64() {
        let v = "a".repeat(64);
        let result = preview(&v);
        assert!(result.starts_with("aa"));
        assert!(result.ends_with("aaaa"));
        assert_eq!(result.chars().filter(|c| *c == '•').count(), 58);
    }

    #[test]
    fn test_preview_multibyte_utf8() {
        // "café" = 4 chars but 5 bytes — old byte-slicing would panic
        let v = "café-secret-token-xyz";
        let result = preview(v); // 21 chars → head 2 + 15 bullets + last 4
        assert!(result.starts_with("ca"));
        assert!(result.ends_with("-xyz"));
    }
}
