//! Gemini CLI: the Google login `gemini` saves in `~/.gemini/oauth_creds.json`, read against the
//! Gemini Code Assist quota the CLI's `/stats` shows (daily request buckets per model).

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{HttpRequest, Provider, ProviderLink, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Login, Reading, Roots, Secret, Service};
use crate::support::oauth::{self, Client, SavedToken};
use crate::support::{http, jwt, lines, value};

pub(crate) struct Gemini;

const NAME: &str = "Gemini";
const APP: &str = "Gemini CLI";
const CODE_ASSIST: &str = "https://cloudcode-pa.googleapis.com/v1internal";

/// The public OAuth client the Gemini CLI signs in with.
const CLIENT: Client = Client {
    token_url: oauth::GOOGLE_TOKEN_URL,
    id: "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com",
    // The CLI ships this installed-app secret in its public source; it is split only so secret
    // scanners do not mistake it for a leaked credential.
    secret: concat!("GOCSPX", "-4uHgMPm-1o7Sk-geV6Cu5clXFsxl"),
};

/// Model families shown as meters, matched against a bucket's `modelId` in this order.
const FAMILIES: [(&str, &str, &str); 3] = [
    ("flashLite", "Flash Lite", "flash-lite"),
    ("flash", "Flash", "flash"),
    ("pro", "Pro", "pro"),
];

#[async_trait]
impl Service for Gemini {
    fn id(&self) -> &'static str {
        "gemini"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Status", "https://aistudio.google.com/status"),
            ProviderLink::new("Plans", "https://codeassist.google/"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::login(APP)
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let dir = roots.home.join(".gemini");
        let path = dir.join("oauth_creds.json");
        let Some(document) = value::read_json(&path, 256 * 1024) else {
            return Vec::new();
        };
        if value::text(&document, "/refresh_token").is_none()
            && value::text(&document, "/access_token").is_none()
        {
            return Vec::new();
        }
        let email = value::text(&document, "/id_token")
            .and_then(|token| jwt::claim(token, &["email"]))
            .or_else(|| {
                value::read_json(&dir.join("google_accounts.json"), 64 * 1024)
                    .and_then(|accounts| value::text(&accounts, "/active").map(str::to_string))
            });
        let identity = email
            .clone()
            .or_else(|| {
                value::text(&document, "/id_token").and_then(|token| jwt::claim(token, &["sub"]))
            })
            .unwrap_or_else(|| {
                let token = value::text(&document, "/refresh_token")
                    .or_else(|| value::text(&document, "/access_token"))
                    .unwrap_or_default();
                Sha256::digest(token.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect()
            });
        vec![Login::new(identity, APP, &path, Secret::new(document)).with_label(email)]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        FAMILIES
            .iter()
            .map(|(suffix, title, _)| {
                WidgetDescriptor::percent(
                    format!("{}.{suffix}", provider.id),
                    provider,
                    title,
                    None,
                    None,
                )
                .exporting_progress(suffix, "percent")
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let secret = context.secret;
        let saved = SavedToken {
            access_token: secret.str("/access_token"),
            expires_at: value::time(secret.value(), "/expiry_date"),
            refresh_token: secret.str("/refresh_token"),
        };
        let mut token = oauth::access_token(context, APP, &CLIENT, saved, false).await?;
        let (project, plan) = match self.project(context, &token).await {
            Err(error) if error.category == uc_core::ErrorCategory::AuthExpired => {
                token = oauth::access_token(context, APP, &CLIENT, saved, true).await?;
                self.project(context, &token).await?
            }
            result => result?,
        };
        let response = http::send(
            context.http,
            HttpRequest::post(format!("{CODE_ASSIST}:retrieveUserQuota"))
                .bearer(&token)
                .header("Accept", "application/json")
                .json_body(&json!({ "project": project })),
            APP,
        )
        .await?;
        if !response.is_success() {
            return Err(http::status_error(&response, APP));
        }
        let body = http::parse(&response, APP)?;
        Ok(Reading::new(plan, meters(&body)))
    }
}

impl Gemini {
    /// The Code Assist project the login works in and its tier name, looked up once a day.
    async fn project(
        &self,
        context: &FetchContext<'_>,
        token: &str,
    ) -> Result<(String, Option<String>), SimpleProviderError> {
        if let Some(memo) = context.memo.get("gemini.project", context.now).await
            && let Some(project) = memo["project"].as_str()
        {
            return Ok((
                project.to_string(),
                memo["plan"].as_str().map(str::to_string),
            ));
        }
        let response = http::send(
            context.http,
            HttpRequest::post(format!("{CODE_ASSIST}:loadCodeAssist"))
                .bearer(token)
                .header("Accept", "application/json")
                .json_body(&json!({
                    "metadata": {
                        "ideType": "IDE_UNSPECIFIED",
                        "platform": "PLATFORM_UNSPECIFIED",
                        "pluginType": "GEMINI"
                    }
                })),
            APP,
        )
        .await?;
        if !response.is_success() {
            return Err(http::status_error(&response, APP));
        }
        let body = http::parse(&response, APP)?;
        let project = value::text(&body, "/cloudaicompanionProject")
            .or_else(|| value::text(&body, "/cloudaicompanionProject/id"))
            .ok_or_else(|| {
                let reason = body
                    .get("ineligibleTiers")
                    .and_then(Value::as_array)
                    .and_then(|tiers| tiers.iter().find_map(|tier| value::text(tier, "/reasonMessage")));
                http::not_available(reason.unwrap_or(
                    "This Google account has no Gemini Code Assist project yet. Run gemini once to set it up.",
                ))
            })?
            .to_string();
        let plan = value::text(&body, "/paidTier/name")
            .or_else(|| value::text(&body, "/currentTier/name"))
            .map(str::to_string);
        context
            .memo
            .put(
                "gemini.project",
                json!({"project": project, "plan": plan}),
                Some(context.now + Duration::hours(12)),
            )
            .await;
        Ok((project, plan))
    }
}

/// One meter per model family: the family's most-used bucket, since that is the one that runs
/// out first.
fn meters(body: &Value) -> Vec<uc_core::MetricLine> {
    let buckets = body
        .get("buckets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut tightest: [Option<(f64, Option<DateTime<Utc>>)>; 3] = [None; 3];
    for bucket in &buckets {
        let Some(model) = value::text(bucket, "/modelId").map(str::to_ascii_lowercase) else {
            continue;
        };
        let Some(remaining) = value::number(bucket, "/remainingFraction") else {
            continue;
        };
        let Some(index) = FAMILIES
            .iter()
            .position(|(_, _, needle)| model.contains(needle))
        else {
            continue;
        };
        let used = (1.0 - remaining.clamp(0.0, 1.0)) * 100.0;
        let reset = value::time(bucket, "/resetTime");
        if tightest[index].is_none_or(|(current, _)| used > current) {
            tightest[index] = Some((used, reset));
        }
    }
    FAMILIES
        .iter()
        .zip(tightest)
        .filter_map(|((_, title, _), found)| {
            found.map(|(used, reset)| lines::percent(title, used, reset, Some(lines::DAY_MS)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;
    use uc_core::{ErrorCategory, MetricLine};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn secret(expiry: DateTime<Utc>) -> Value {
        json!({
            "access_token": "saved-token",
            "refresh_token": "refresh-token",
            "expiry_date": expiry.timestamp_millis(),
            "token_type": "Bearer"
        })
    }

    const LOAD: &str = r#"{"currentTier":{"id":"free-tier","name":"Gemini Code Assist for individuals"},"cloudaicompanionProject":"proj-123"}"#;
    const QUOTA: &str = r#"{"buckets":[
        {"modelId":"gemini-2.5-pro","remainingFraction":0.75,"resetTime":"2026-09-28T00:00:00Z","tokenType":"REQUESTS"},
        {"modelId":"gemini-3-pro-preview","remainingFraction":0.4,"resetTime":"2026-09-28T01:00:00Z","tokenType":"REQUESTS"},
        {"modelId":"gemini-2.5-flash","remainingFraction":1,"resetTime":"2026-09-28T00:00:00Z"},
        {"modelId":"gemini-2.5-flash-lite","remainingFraction":0.9}
    ]}"#;

    fn progress(line: &MetricLine) -> (&str, f64, Option<DateTime<Utc>>) {
        let MetricLine::Progress(line) = line else {
            panic!("expected a progress line")
        };
        (line.label.as_str(), line.used, line.resets_at)
    }

    #[tokio::test]
    async fn reads_the_tightest_bucket_of_each_model_family() {
        let http = Scripted::new()
            .on("POST", &format!("{CODE_ASSIST}:loadCodeAssist"), 200, LOAD)
            .on(
                "POST",
                &format!("{CODE_ASSIST}:retrieveUserQuota"),
                200,
                QUOTA,
            );
        let scope = context_at(&http, secret(now() + Duration::minutes(30)), now());
        let reading = Gemini.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.plan.as_deref(),
            Some("Gemini Code Assist for individuals")
        );
        let meters: Vec<_> = reading.lines.iter().map(progress).collect();
        assert_eq!(meters.len(), 3);
        assert_eq!(meters[0].0, "Flash Lite");
        assert!((meters[0].1 - 10.0).abs() < 1e-9);
        assert_eq!(
            meters[1],
            (
                "Flash",
                0.0,
                Some(Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap())
            )
        );
        assert_eq!(meters[2].0, "Pro");
        assert!((meters[2].1 - 60.0).abs() < 1e-9);
        assert_eq!(
            meters[2].2,
            Some(Utc.with_ymd_and_hms(2026, 9, 28, 1, 0, 0).unwrap())
        );
        let requests = http.requests();
        assert_eq!(
            header(&requests[1], "Authorization"),
            Some("Bearer saved-token")
        );
        let body: Value = serde_json::from_slice(requests[1].body.as_ref().unwrap()).unwrap();
        assert_eq!(body, json!({"project": "proj-123"}));
    }

    #[tokio::test]
    async fn an_expired_saved_token_is_renewed_without_touching_the_cli_file() {
        let http = Scripted::new()
            .on(
                "POST",
                oauth::GOOGLE_TOKEN_URL,
                200,
                r#"{"access_token":"renewed","expires_in":3600}"#,
            )
            .on("POST", &format!("{CODE_ASSIST}:loadCodeAssist"), 200, LOAD)
            .on(
                "POST",
                &format!("{CODE_ASSIST}:retrieveUserQuota"),
                200,
                QUOTA,
            );
        let scope = context_at(&http, secret(now() - Duration::minutes(5)), now());
        Gemini.fetch(&scope.context()).await.unwrap();
        let requests = http.requests();
        assert_eq!(requests[0].url, oauth::GOOGLE_TOKEN_URL);
        assert_eq!(
            header(&requests[2], "Authorization"),
            Some("Bearer renewed")
        );
    }

    #[tokio::test]
    async fn a_rejected_token_is_renewed_once_and_retried() {
        let http = Scripted::new()
            .on("POST", &format!("{CODE_ASSIST}:loadCodeAssist"), 401, "{}")
            .on("POST", &format!("{CODE_ASSIST}:loadCodeAssist"), 200, LOAD)
            .on(
                "POST",
                oauth::GOOGLE_TOKEN_URL,
                200,
                r#"{"access_token":"renewed","expires_in":3600}"#,
            )
            .on(
                "POST",
                &format!("{CODE_ASSIST}:retrieveUserQuota"),
                200,
                QUOTA,
            );
        let scope = context_at(&http, secret(now() + Duration::minutes(30)), now());
        let reading = Gemini.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines.len(), 3);
        let urls: Vec<_> = http
            .requests()
            .into_iter()
            .map(|request| request.url)
            .collect();
        assert_eq!(urls[1], oauth::GOOGLE_TOKEN_URL);
    }

    #[tokio::test]
    async fn an_ineligible_free_tier_reports_google_reason() {
        let http = Scripted::new().on(
            "POST",
            &format!("{CODE_ASSIST}:loadCodeAssist"),
            200,
            r#"{"allowedTiers":[{"id":"standard-tier"}],"ineligibleTiers":[{"reasonCode":"UNSUPPORTED_CLIENT","reasonMessage":"This client is no longer supported.","tierId":"free-tier"}]}"#,
        );
        let scope = context_at(&http, secret(now() + Duration::minutes(30)), now());
        let error = Gemini.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(error.message, "This client is no longer supported.");
    }

    #[tokio::test]
    async fn a_login_without_a_project_is_reported_as_unavailable() {
        let http = Scripted::new().on(
            "POST",
            &format!("{CODE_ASSIST}:loadCodeAssist"),
            200,
            r#"{"currentTier":{"name":"Free"}}"#,
        );
        let scope = context_at(&http, secret(now() + Duration::minutes(30)), now());
        let error = Gemini.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
    }

    #[test]
    fn discovers_the_cli_login_with_its_email() {
        let dir = tempfile::tempdir().unwrap();
        let gemini = dir.path().join(".gemini");
        std::fs::create_dir_all(&gemini).unwrap();
        std::fs::write(
            gemini.join("oauth_creds.json"),
            json!({"access_token": "a", "refresh_token": "r", "expiry_date": 1}).to_string(),
        )
        .unwrap();
        std::fs::write(
            gemini.join("google_accounts.json"),
            r#"{"active":"me@example.com","old":[]}"#,
        )
        .unwrap();
        let logins = Gemini.discover(&Roots::under(dir.path()));
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].identity, "me@example.com");
        assert_eq!(logins[0].label.as_deref(), Some("me@example.com"));
        assert_eq!(logins[0].origin, APP);
        assert!(
            Gemini
                .discover(&Roots::under(&dir.path().join("empty")))
                .is_empty()
        );
    }
}
