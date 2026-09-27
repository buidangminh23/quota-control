//! Codebuff: the credits a Codebuff account has used of its quota, the credits left, the credits
//! used of its subscription's weekly limit, and the plan's name.
//!
//! The login Codebuff keeps in `~/.config/manicode/credentials.json` (the `authToken` of its
//! `default` profile, else a top-level `authToken`) is read from the home folder on Windows, macOS
//! and Linux alike; a key can instead come from the `CODEBUFF_API_KEY` environment variable or be
//! saved in Quota Control. A refresh sends two requests with the token as a bearer token: a
//! read-only `POST https://www.codebuff.com/api/v1/usage` with the JSON body
//! `{"fingerprintId":"codexbar-usage"}`, and then
//! `GET https://www.codebuff.com/api/user/subscription`. When the subscription request fails, the
//! card still shows the credits, without the plan and the Weekly row.

use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{HttpRequest, MetricKind, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::support::{http, lines, value};

pub(crate) struct Codebuff;

const NAME: &str = "Codebuff";
const URL: &str = "https://www.codebuff.com/api/v1/usage";

#[async_trait]
impl Service for Codebuff {
    fn id(&self) -> &'static str {
        "codebuff"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::login("Codebuff").or_api_key(ApiKeyHelp {
            env: &["CODEBUFF_API_KEY"],
            url: "https://www.codebuff.com",
            fields: &[],
        })
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let path = roots.home.join(".config/manicode/credentials.json");
        let Some(document) = value::read_json(&path, 1024 * 1024) else {
            return Vec::new();
        };
        let Some(key) = value::text(&document, "/default/authToken")
            .or_else(|| value::text(&document, "/authToken"))
        else {
            return Vec::new();
        };
        let identity = Sha256::digest(key.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        vec![Login::new(
            identity,
            "Codebuff",
            &path,
            Secret::api_key(key),
        )]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::bounded_count(
                format!("{}.credits", provider.id),
                provider,
                "Credits",
                None,
                0.0,
                "credits",
                None,
            ),
            WidgetDescriptor::values(
                format!("{}.balance", provider.id),
                provider,
                "Balance",
                None,
                Some(MetricKind::Count),
                Some("left"),
                false,
                None,
                false,
            ),
            WidgetDescriptor::bounded_count(
                format!("{}.weekly", provider.id),
                provider,
                "Weekly",
                None,
                0.0,
                "credits",
                Some(lines::WEEK_MS),
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Codebuff API key is missing."))?;
        let body = http::json(
            context.http,
            HttpRequest::post(URL)
                .bearer(key)
                .header("Accept", "application/json")
                .json_body(&json!({"fingerprintId":"codexbar-usage"})),
            NAME,
        )
        .await?;
        let mut reading = parse(&body)?;
        if let Ok(subscription) = http::json(
            context.http,
            HttpRequest::get("https://www.codebuff.com/api/user/subscription").bearer(key),
            NAME,
        )
        .await
        {
            reading.plan = value::text(&subscription, "/subscription/displayName")
                .or_else(|| value::text(&subscription, "/subscription/tier"))
                .or_else(|| value::text(&subscription, "/tier"))
                .and_then(lines::plan_name);
            if let (Some(used), Some(limit)) = (
                value::number(&subscription, "/rateLimit/weeklyUsed")
                    .or_else(|| value::number(&subscription, "/rateLimit/used")),
                value::number(&subscription, "/rateLimit/weeklyLimit")
                    .or_else(|| value::number(&subscription, "/rateLimit/limit")),
            ) {
                reading.lines.push(lines::count(
                    "Weekly",
                    used,
                    limit,
                    "credits",
                    value::time(&subscription, "/rateLimit/weeklyResetsAt"),
                    Some(lines::WEEK_MS),
                ));
            }
        }
        Ok(reading)
    }
}

/// The usage answer as the Credits and Balance rows.
fn parse(body: &Value) -> Result<Reading, SimpleProviderError> {
    let used = value::number(body, "/usage").or_else(|| value::number(body, "/used"));
    let total = value::number(body, "/quota").or_else(|| value::number(body, "/limit"));
    let left =
        value::number(body, "/remainingBalance").or_else(|| value::number(body, "/remaining"));
    let mut rows = Vec::new();
    if let (Some(used), Some(total)) = (used, total) {
        rows.push(lines::count(
            "Credits",
            used,
            total,
            "credits",
            value::time(body, "/next_quota_reset"),
            None,
        ));
    }
    if let Some(left) = left {
        rows.push(lines::count_value("Balance", left, "credits"));
    }
    if rows.is_empty() {
        return Err(http::decoding(NAME));
    }
    Ok(Reading::new(None, rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use uc_core::ErrorCategory;

    #[test]
    fn the_token_in_the_manicode_credentials_becomes_one_login_under_its_hash() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        assert!(Codebuff.discover(&roots).is_empty());
        let path = dir.path().join(".config/manicode");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("credentials.json"),
            r#"{"default":{"authToken":"fake"}}"#,
        )
        .unwrap();
        let found = Codebuff.discover(&roots);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].identity.len(), 64);
        assert_eq!(found[0].secret.key(), Some("fake"));
    }

    #[tokio::test]
    async fn posts_the_fingerprint_and_reads_the_credits_even_when_the_subscription_fails() {
        let body = r###"{"usage":200,"quota":1000,"remainingBalance":800,"next_quota_reset":"2026-10-01T00:00:00Z"}"###;
        let http = Scripted::new().on("POST", URL, 200, body);
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey":"test"}), now);
        let reading = Codebuff.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                None,
                vec![
                    lines::count(
                        "Credits",
                        200.0,
                        1000.0,
                        "credits",
                        Some(Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()),
                        None
                    ),
                    lines::count_value("Balance", 800.0, "credits")
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url, URL);
        assert_eq!(header(&requests[0], "Authorization"), Some("Bearer test"));
        assert_eq!(
            serde_json::from_slice::<Value>(requests[0].body.as_ref().unwrap()).unwrap(),
            json!({"fingerprintId":"codexbar-usage"})
        );
    }

    #[tokio::test]
    async fn failed_or_unreadable_answers_keep_their_categories_without_echoing_the_body() {
        for (status, body, category) in [
            (401, "secret", ErrorCategory::AuthExpired),
            (403, "secret", ErrorCategory::AuthExpired),
            (429, "", ErrorCategory::RateLimited),
            (500, "", ErrorCategory::Http5xx),
            (200, "{}", ErrorCategory::Decoding),
            (200, "bad json", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("POST", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            let error = Codebuff.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            Codebuff.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
