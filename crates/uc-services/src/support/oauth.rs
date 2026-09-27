//! Renewing an OAuth access token in memory for services whose app keeps a refresh token that does
//! not rotate (Google's installed-app logins: Gemini CLI, Antigravity). The renewed token lives
//! only in the card's memo: the app's own file is never written, so the app stays signed in and
//! renews its own copy as usual.

use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use uc_core::{ErrorCategory, HttpRequest, SimpleProviderError};

use crate::service::FetchContext;
use crate::support::{http, value};

/// A public OAuth client, as the app itself ships it.
#[derive(Clone, Copy, Debug)]
pub struct Client {
    pub token_url: &'static str,
    pub id: &'static str,
    /// Empty for clients without a secret.
    pub secret: &'static str,
}

pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// How long before its stated expiry a token is treated as expired.
const LEEWAY_SECONDS: i64 = 60;

/// What to do when a sign-in made in Quota Control lapses: the card names the service already.
pub const SIGN_IN_EXPIRED: &str = "This sign-in expired. Sign in again in Accounts.";
pub const SIGN_IN_REVOKED: &str =
    "This sign-in can no longer be renewed. Sign in again in Accounts.";

/// What the app's saved login holds.
#[derive(Clone, Copy, Debug)]
pub struct SavedToken<'a> {
    pub access_token: Option<&'a str>,
    pub expires_at: Option<DateTime<Utc>>,
    pub refresh_token: Option<&'a str>,
}

/// A usable access token: the saved one while it is fresh, else one renewed earlier for this
/// card, else a new one from `client`. `force` skips the saved and remembered tokens, for a retry
/// after the service rejected them.
pub async fn access_token(
    context: &FetchContext<'_>,
    service: &str,
    client: &Client,
    saved: SavedToken<'_>,
    force: bool,
) -> Result<String, SimpleProviderError> {
    let now = context.now;
    let fresh = |expires: Option<DateTime<Utc>>| {
        expires.is_none_or(|expires| expires > now + Duration::seconds(LEEWAY_SECONDS))
    };
    if !force
        && let Some(token) = saved.access_token
        && fresh(saved.expires_at)
    {
        return Ok(token.to_string());
    }
    let owned = context.secret.is_owned();
    let Some(refresh_token) = saved.refresh_token else {
        return Err(SimpleProviderError::new(
            ErrorCategory::AuthExpired,
            if owned {
                SIGN_IN_EXPIRED.to_string()
            } else {
                format!("The {service} login expired. Open {service} once to renew it.")
            },
        ));
    };
    let fingerprint: String = Sha256::digest(refresh_token.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if !force
        && let Some(memo) = context.memo.get("oauth.renewed", now).await
        && memo["for"] == fingerprint
        && let Some(token) = memo["token"].as_str()
    {
        return Ok(token.to_string());
    }
    let mut fields = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client.id),
    ];
    if !client.secret.is_empty() {
        fields.push(("client_secret", client.secret));
    }
    let response = http::send(
        context.http,
        HttpRequest::post(client.token_url)
            .form_body(&fields)
            .header("Accept", "application/json"),
        service,
    )
    .await?;
    if matches!(response.status, 400 | 401 | 403) {
        return Err(SimpleProviderError::new(
            ErrorCategory::AuthExpired,
            if owned {
                SIGN_IN_REVOKED.to_string()
            } else {
                format!("The {service} login can no longer be renewed. Sign in to {service} again.")
            },
        ));
    }
    if !response.is_success() {
        return Err(http::status_error(&response, service));
    }
    let body = http::parse(&response, service)?;
    let token = value::text(&body, "/access_token")
        .ok_or_else(|| http::decoding(service))?
        .to_string();
    let lifetime = value::number(&body, "/expires_in").unwrap_or(3600.0) as i64;
    let expires = now + Duration::seconds((lifetime - LEEWAY_SECONDS).max(60));
    context
        .memo
        .put(
            "oauth.renewed",
            json!({"for": fingerprint, "token": token}),
            Some(expires),
        )
        .await;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at};
    use chrono::TimeZone;

    const CLIENT: Client = Client {
        token_url: GOOGLE_TOKEN_URL,
        id: "client-id",
        secret: "client-secret",
    };

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn a_fresh_saved_token_is_used_as_is() {
        let http = Scripted::new();
        let scope = context_at(&http, serde_json::Value::Null, now());
        let token = access_token(
            &scope.context(),
            "Gemini CLI",
            &CLIENT,
            SavedToken {
                access_token: Some("saved"),
                expires_at: Some(now() + Duration::minutes(30)),
                refresh_token: Some("refresh"),
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(token, "saved");
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn an_expired_token_is_renewed_once_and_remembered() {
        let http = Scripted::new().on(
            "POST",
            GOOGLE_TOKEN_URL,
            200,
            r#"{"access_token":"renewed","expires_in":3599}"#,
        );
        let scope = context_at(&http, serde_json::Value::Null, now());
        let saved = SavedToken {
            access_token: Some("old"),
            expires_at: Some(now() - Duration::minutes(1)),
            refresh_token: Some("refresh"),
        };
        for _ in 0..2 {
            let token = access_token(&scope.context(), "Gemini CLI", &CLIENT, saved, false)
                .await
                .unwrap();
            assert_eq!(token, "renewed");
        }
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        let body = String::from_utf8(requests[0].body.clone().unwrap()).unwrap();
        assert!(body.contains("grant_type=refresh_token"));
        assert!(body.contains("client_secret=client-secret"));
    }

    #[tokio::test]
    async fn a_refused_renewal_asks_to_sign_in_again() {
        let http = Scripted::new().on(
            "POST",
            GOOGLE_TOKEN_URL,
            400,
            r#"{"error":"invalid_grant"}"#,
        );
        let scope = context_at(&http, serde_json::Value::Null, now());
        let error = access_token(
            &scope.context(),
            "Gemini CLI",
            &CLIENT,
            SavedToken {
                access_token: None,
                expires_at: None,
                refresh_token: Some("refresh"),
            },
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
    }
}
