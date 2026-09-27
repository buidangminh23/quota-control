//! Signing in to a Google account the way Google's own apps do (the Gemini CLI, Antigravity): the
//! authorization parameters they send, and the account a token answer belongs to.

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use uc_core::{HttpRequest, SharedHttpClient, SimpleProviderError};

use crate::signin;
use crate::support::{jwt, value};

pub const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";

pub const CLOUD_PLATFORM: &str = "https://www.googleapis.com/auth/cloud-platform";
pub const EMAIL: &str = "https://www.googleapis.com/auth/userinfo.email";
pub const PROFILE: &str = "https://www.googleapis.com/auth/userinfo.profile";

/// A refresh token that outlives the hour-long access token (`consent` makes Google send one even
/// to an account that signed in to the app before), and Google's account chooser, so the user
/// picks which account to add.
pub const PARAMS: &[(&str, &str)] = &[
    ("access_type", "offline"),
    ("prompt", "select_account consent"),
];

/// The lasting tokens of a Google sign-in and the account they belong to.
#[derive(Debug)]
pub struct Account {
    pub email: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: DateTime<Utc>,
    pub id_token: Option<String>,
}

/// The account a token answer belongs to: the email in the ID token Google sent with it, else the
/// one the userinfo endpoint reports. The ID token came straight from Google's token endpoint, so
/// it is read without checking its signature, as Google's own tools do.
pub async fn account(
    http: &SharedHttpClient,
    answer: &Value,
    product: &str,
) -> Result<Account, SimpleProviderError> {
    let access_token = value::text(answer, "/access_token")
        .ok_or_else(|| signin::invalid("Google returned no access token."))?
        .to_string();
    let refresh_token = value::text(answer, "/refresh_token")
        .ok_or_else(|| {
            signin::invalid("Google did not grant a lasting sign-in. Start a new sign-in.")
        })?
        .to_string();
    let lifetime = value::number(answer, "/expires_in")
        .unwrap_or(3600.0)
        .clamp(60.0, 86_400.0) as i64;
    let id_token = value::text(answer, "/id_token").map(str::to_string);
    let email = match id_token
        .as_deref()
        .and_then(|token| jwt::claim(token, &["email"]))
    {
        Some(email) => email,
        None => userinfo_email(http, &access_token, product).await?,
    };
    Ok(Account {
        email,
        access_token,
        refresh_token,
        expires_at: Utc::now() + Duration::seconds(lifetime),
        id_token,
    })
}

async fn userinfo_email(
    http: &SharedHttpClient,
    token: &str,
    product: &str,
) -> Result<String, SimpleProviderError> {
    let response = http
        .send(
            HttpRequest::get(USERINFO_URL)
                .bearer(token)
                .header("Accept", "application/json")
                .timeout(signin::REQUEST_TIMEOUT),
        )
        .await
        .map_err(|_| signin::network(product))?;
    let body: Value = if response.is_success() {
        response.json().unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    value::text(&body, "/email")
        .map(str::to_string)
        .ok_or_else(|| signin::invalid("Google did not say which account signed in."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, header};
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde_json::json;

    fn id_token(claims: Value) -> String {
        let part = |value: &Value| URL_SAFE_NO_PAD.encode(value.to_string());
        format!(
            "{}.{}.signature",
            part(&json!({"alg": "RS256"})),
            part(&claims)
        )
    }

    #[tokio::test]
    async fn the_id_token_names_the_account_without_another_request() {
        let http = Scripted::new();
        let answer = json!({
            "access_token": "ya29.token",
            "refresh_token": "1//refresh",
            "expires_in": 3599,
            "id_token": id_token(json!({"email": "minh@example.com", "sub": "1"})),
        });
        let account = account(&http.shared(), &answer, "Gemini").await.unwrap();
        assert_eq!(account.email, "minh@example.com");
        assert_eq!(account.refresh_token, "1//refresh");
        assert!(account.expires_at > Utc::now() + Duration::minutes(58));
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn without_an_id_token_the_userinfo_endpoint_names_the_account() {
        let http = Scripted::new().on(
            "GET",
            USERINFO_URL,
            200,
            r#"{"email":"minh@example.com","name":"Minh"}"#,
        );
        let answer = json!({"access_token": "ya29.token", "refresh_token": "1//refresh"});
        let account = account(&http.shared(), &answer, "Antigravity")
            .await
            .unwrap();
        assert_eq!(account.email, "minh@example.com");
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some("Bearer ya29.token")
        );
    }

    #[tokio::test]
    async fn a_sign_in_without_a_refresh_token_is_refused() {
        let http = Scripted::new();
        let answer = json!({"access_token": "ya29.token", "expires_in": 3599});
        let error = account(&http.shared(), &answer, "Gemini")
            .await
            .unwrap_err();
        assert!(error.message.contains("lasting sign-in"));
    }
}
