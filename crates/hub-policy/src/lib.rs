//! Conservative journal sanitization. Incremental deltas are not persisted as text.
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

pub fn redact(text: &str) -> String {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| [
        r"(?i)\b(?:sk-[a-z0-9_-]{8,}|gh[pousr]_[a-z0-9_]{8,}|github_pat_[a-z0-9_]{8,}|AKIA[A-Z0-9]{16})\b",
        r"(?i)\bBearer\s+[a-z0-9._~+/=-]+",
        r#"(?i)(?:api[_-]?key|access[_-]?token|refresh[_-]?token|password|secret|authorization)\s*[=:]\s*["']?[^\s,"';}]+"#,
        r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
    ].iter().map(|p| Regex::new(p).expect("static redaction regex")).collect());
    patterns.iter().fold(text.to_owned(), |s, p| {
        p.replace_all(&s, "[REDACTED]").into_owned()
    })
}
pub fn redact_json(value: &mut Value) {
    match value {
        Value::String(s) => *s = redact(s),
        Value::Array(v) => v.iter_mut().for_each(redact_json),
        Value::Object(m) => {
            for (key, value) in m {
                let lower = key.to_ascii_lowercase();
                if [
                    "apikey",
                    "api_key",
                    "accesstoken",
                    "access_token",
                    "refreshtoken",
                    "refresh_token",
                    "password",
                    "secret",
                    "authorization",
                    "token",
                ]
                .contains(&lower.as_str())
                {
                    *value = Value::String("[REDACTED]".into());
                } else {
                    redact_json(value);
                }
            }
        }
        _ => {}
    }
}

pub fn scope_digest(goal: &str, diff: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update((goal.len() as u64).to_le_bytes());
    hash.update(goal.as_bytes());
    hash.update(diff.as_bytes());
    format!("{:x}", hash.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secrets_removed_but_code_retained() {
        let s = redact("api_key=1234 sk-abcdefgh123456 Bearer xyz.abc\nfn main() {}");
        assert!(!s.contains("1234"));
        assert!(!s.contains("xyz.abc"));
        assert!(s.contains("fn main()"));
    }
    #[test]
    fn nested_credentials_removed() {
        let mut v = serde_json::json!({"nested":{"access_token":"plain-secret"},"ok":"code"});
        redact_json(&mut v);
        assert!(!v.to_string().contains("plain-secret"));
    }
}
