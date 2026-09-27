//! Reading the claims of a JSON Web Token without verifying it: only to tell accounts apart and to
//! know when a token expires. Nothing here trusts a claim for access decisions.

use base64::Engine;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde_json::Value;

/// The payload of `token`, when it is a JWT with a JSON object payload.
pub fn claims(token: &str) -> Option<Value> {
    let payload = token.trim().split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .or_else(|_| URL_SAFE.decode(payload))
        .ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value.is_object().then_some(value)
}

/// When `token` expires, from its `exp` claim.
pub fn expires_at(token: &str) -> Option<DateTime<Utc>> {
    let seconds = claims(token)?.get("exp")?.as_f64()?;
    DateTime::from_timestamp(seconds as i64, 0)
}

/// The first non-empty string claim among `names`.
pub fn claim(token: &str, names: &[&str]) -> Option<String> {
    let claims = claims(token)?;
    names.iter().find_map(|name| {
        claims
            .get(*name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(payload: &str) -> String {
        format!("h.{}.s", URL_SAFE_NO_PAD.encode(payload))
    }

    #[test]
    fn reads_claims_and_expiry() {
        let jwt = token(r#"{"sub":"u-1","email":"a@b.c","exp":1790503200}"#);
        assert_eq!(claim(&jwt, &["email", "sub"]).as_deref(), Some("a@b.c"));
        assert_eq!(expires_at(&jwt).unwrap().timestamp(), 1790503200);
    }

    #[test]
    fn rejects_non_tokens() {
        assert!(claims("not-a-token").is_none());
        assert!(claims(&token("[1,2]")).is_none());
    }
}
