//! Log redaction. Port of upstream `LogRedaction.swift` (itself a port of the Tauri host API).
//!
//! `log_message` is the lightweight last line of defense for free-form log lines; any caller logging
//! a URL or a response body must pre-redact it with `url` or `body_preview`. The path pass also
//! covers Windows profile paths (`C:\Users\…`), which upstream never saw.

use std::sync::LazyLock;

use regex::{Captures, Regex};

/// Redact a sensitive value to `first4...last4`, or `[REDACTED]` when it is too short to mask.
pub fn value(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 12 {
        return "[REDACTED]".to_string();
    }
    let first: String = chars[..4].iter().collect();
    let last: String = chars[chars.len() - 4..].iter().collect();
    format!("{first}...{last}")
}

const URL_SENSITIVE_PARAMS: &[&str] = &[
    "key", "api_key", "apikey", "token", "access_token", "secret", "password", "auth", "authorization", "bearer",
    "credential", "user", "user_id", "userid", "account_id", "accountid", "profilearn", "profile_arn", "email",
    "login",
];

/// Redact sensitive query parameters in a URL; the path is left intact.
pub fn url(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let params: Vec<String> = query
        .split('&')
        .map(|param| match param.split_once('=') {
            Some((name, val)) if !val.is_empty() => {
                let lower = name.to_ascii_lowercase();
                if URL_SENSITIVE_PARAMS.iter().any(|needle| lower.contains(needle)) {
                    format!("{name}={}", value(val))
                } else {
                    param.to_string()
                }
            }
            _ => param.to_string(),
        })
        .collect();
    format!("{base}?{}", params.join("&"))
}

const JSON_SENSITIVE_KEYS: &[&str] = &[
    "name", "password", "token", "access_token", "refresh_token", "secret", "api_key", "apiKey", "authorization",
    "bearer", "credential", "session_token", "sessionToken", "auth_token", "authToken", "id_token", "idToken",
    "accessToken", "refreshToken", "user_id", "userId", "account_id", "accountId", "team_id", "teamId", "org_id",
    "orgId", "account_display_name", "accountDisplayName", "payment_id", "paymentId", "profile_arn", "profileArn",
    "email", "login", "analytics_tracking_id",
];

static JWT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").unwrap());
static API_KEY_QUOTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"["']?(sk-|pk-|api_|key_|secret_)[A-Za-z0-9_-]{12,}["']?"#).unwrap());
static API_KEY_BARE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(sk-|pk-|api_|key_|secret_)[A-Za-z0-9_-]{12,}").unwrap());
static DEVIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"devin-session-token\$[^\s"',}\]]+"#).unwrap());
static ACCOUNT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(account=)([^,\s]+)").unwrap());
static PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(/(?:Users|home|opt|private|var|tmp|Applications)/[^\s"')]+|[A-Za-z]:\\(?:Users|ProgramData|Program Files|Windows)\\[^\s"')]+)"#,
    )
    .unwrap()
});
static JSON_KEYS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    JSON_SENSITIVE_KEYS
        .iter()
        .map(|key| (*key, Regex::new(&format!(r#""{}":\s*"([^"]+)""#, regex::escape(key))).unwrap()))
        .collect()
});

/// Redact sensitive patterns in a response body: JWT, quoted api key, Devin session, sensitive JSON
/// keys, then filesystem paths. Redacts before any truncation.
pub fn body(body: &str) -> String {
    let mut result = JWT.replace_all(body, |c: &Captures| value(&c[0])).into_owned();
    result = API_KEY_QUOTED
        .replace_all(&result, |c: &Captures| value(c[0].trim_matches(|ch| ch == '"' || ch == '\'')))
        .into_owned();
    result = DEVIN.replace_all(&result, |c: &Captures| value(&c[0])).into_owned();
    for (key, regex) in JSON_KEYS.iter() {
        result = regex.replace_all(&result, |c: &Captures| format!("\"{key}\": \"{}\"", value(&c[1]))).into_owned();
    }
    PATH.replace_all(&result, "[PATH]").into_owned()
}

/// Redact a body, then truncate to a char-safe preview with a byte-count suffix.
pub fn body_preview(raw: &str) -> String {
    body_preview_with_limit(raw, 500)
}

pub fn body_preview_with_limit(raw: &str, limit: usize) -> String {
    let redacted = body(raw);
    if redacted.len() <= limit {
        return redacted;
    }
    let truncated: String = redacted.char_indices().take_while(|(index, _)| *index < limit).map(|(_, ch)| ch).collect();
    format!("{truncated}... ({} bytes total)", raw.len())
}

/// Lightweight redaction for free-form log messages.
pub fn log_message(message: &str) -> String {
    let mut result = JWT.replace_all(message, |c: &Captures| value(&c[0])).into_owned();
    result = API_KEY_BARE.replace_all(&result, |c: &Captures| value(&c[0])).into_owned();
    result = DEVIN.replace_all(&result, |c: &Captures| value(&c[0])).into_owned();
    result = ACCOUNT.replace_all(&result, |c: &Captures| format!("{}{}", &c[1], value(&c[2]))).into_owned();
    PATH.replace_all(&result, "[PATH]").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_long_values_and_hides_short_ones() {
        assert_eq!(value("abcdefghijklmnop"), "abcd...mnop");
        assert_eq!(value("short"), "[REDACTED]");
    }

    #[test]
    fn redacts_sensitive_query_parameters_only() {
        assert_eq!(
            url("https://api.example.com/v1?access_token=abcdefghijklmnop&page=2"),
            "https://api.example.com/v1?access_token=abcd...mnop&page=2"
        );
        assert_eq!(url("https://api.example.com/v1/usage"), "https://api.example.com/v1/usage");
    }

    #[test]
    fn redacts_tokens_and_paths_in_bodies() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.c2lnbmF0dXJlc2lnbmF0dXJl";
        let raw = format!(r#"{{"access_token": "{jwt}", "file": "C:\Users\Minh\secret.json"}}"#);
        let redacted = body(&raw);
        assert!(!redacted.contains(jwt));
        assert!(redacted.contains("[PATH]"));
    }

    #[test]
    fn previews_truncate_after_redaction() {
        let raw = "x".repeat(600);
        let preview = body_preview(&raw);
        assert!(preview.ends_with("... (600 bytes total)"));
    }

    #[test]
    fn log_messages_redact_account_values() {
        assert_eq!(log_message("refresh account=1234567890abcdef ok"), "refresh account=1234...cdef ok");
    }
}
