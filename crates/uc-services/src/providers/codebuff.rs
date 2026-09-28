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
//! card still shows the credits, without the plan and the Weekly row. The subscription answer also
//! names the account's email and, under `subscription`, the end of the billing period.

use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{HttpRequest, MetricKind, PlanTerm, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::signin::{
    self, Converted, Look, Method, Pending, Poll, Polling, SignIn, SignedIn, StartContext,
};
use crate::support::{http, lines, value};

pub(crate) struct Codebuff;

const NAME: &str = "Codebuff";
const URL: &str = "https://www.codebuff.com/api/v1/usage";
/// The browser sign-in of the Codebuff CLI: a login page for this device, and where the app asks
/// whether it was used.
const LOGIN_CODE: &str = "https://www.codebuff.com/api/auth/cli/code";
const LOGIN_STATUS: &str = "https://www.codebuff.com/api/auth/cli/status";

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

    fn sign_in(&self) -> Option<&'static dyn SignIn> {
        Some(&Codebuff)
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
        let email = value::text(&document, "/default/email").map(str::to_string);
        vec![Login::new(identity, "Codebuff", &path, Secret::api_key(key)).with_label(email)]
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
            reading = reading
                .with_account(
                    value::text(&subscription, "/email")
                        .or_else(|| value::text(&subscription, "/user/email")),
                )
                .with_plan_term(
                    value::time(&subscription, "/subscription/billingPeriodEnd")
                        .or_else(|| value::time(&subscription, "/subscription/currentPeriodEnd"))
                        .map(|ends_at| PlanTerm::Stated {
                            ends_at,
                            checked_at: None,
                        }),
                );
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

#[async_trait]
impl SignIn for Codebuff {
    /// Codebuff signs in with GitHub only.
    fn methods(&self) -> &'static [Method] {
        &[Method::GitHub]
    }

    async fn start(
        &self,
        _method: Method,
        context: &StartContext,
    ) -> Result<Pending, SimpleProviderError> {
        let fingerprint = format!("quota-control-{}", signin::random_token(6)?);
        let response = context
            .http
            .send(
                HttpRequest::post(LOGIN_CODE)
                    .json_body(&json!({ "fingerprintId": fingerprint }))
                    .header("Accept", "application/json")
                    .timeout(signin::REQUEST_TIMEOUT),
            )
            .await
            .map_err(|_| signin::network(NAME))?;
        let body: Value = response.json().unwrap_or(Value::Null);
        let page =
            value::text(&body, "/loginUrl").and_then(|raw| signin::page_on(raw, &["codebuff.com"]));
        let hash = value::text(&body, "/fingerprintHash").map(str::to_string);
        let expires = body.get("expiresAt").and_then(|expires| match expires {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        });
        let (Some(page), Some(hash), Some(expires), true) =
            (page, hash, expires, response.is_success())
        else {
            return Err(SimpleProviderError::new(
                uc_core::ErrorCategory::http(response.status),
                format!("{NAME} did not start a sign-in. Try again later."),
            ));
        };
        let mut status = url::Url::parse(LOGIN_STATUS).map_err(|_| http::decoding(NAME))?;
        status
            .query_pairs_mut()
            .append_pair("fingerprintId", &fingerprint)
            .append_pair("fingerprintHash", &hash)
            .append_pair("expiresAt", &expires);
        let status = status.to_string();
        let requests = context.http.clone();
        let look = move || -> Look {
            let http = requests.clone();
            let status = status.clone();
            Box::pin(async move { look_login(&http, &status).await })
        };
        Ok(signin::polled(
            page,
            None,
            Polling {
                interval: std::time::Duration::from_secs(5),
                slow_down: std::time::Duration::from_secs(5),
                lifetime: signin::FLOW_LIFETIME,
            },
            context.http.clone(),
            look,
            signed_in,
        ))
    }
}

/// One look at a browser sign-in: Codebuff answers 401 until the user has signed in on the page,
/// and the CLI keeps asking through any other failure, as this does.
async fn look_login(http: &uc_core::SharedHttpClient, status: &str) -> Poll {
    let Ok(response) = http
        .send(
            HttpRequest::get(status)
                .header("Accept", "application/json")
                .timeout(signin::REQUEST_TIMEOUT),
        )
        .await
    else {
        return Poll::Waiting;
    };
    if response.status == 429 {
        return Poll::SlowDown;
    }
    if !response.is_success() {
        return Poll::Waiting;
    }
    let body: Value = response.json().unwrap_or(Value::Null);
    if body.get("user").is_some_and(Value::is_object) {
        Poll::Approved(body)
    } else {
        Poll::Waiting
    }
}

/// A signed-in CLI session's token, saved as the Codebuff CLI saves it and named by its user.
fn signed_in(_: uc_core::SharedHttpClient, answer: Value) -> Converted {
    Box::pin(async move {
        let user = &answer["user"];
        let token = value::text(user, "/authToken")
            .ok_or_else(|| signin::invalid("Codebuff returned no usable sign-in."))?;
        let identity = value::text(user, "/id")
            .or_else(|| value::text(user, "/email"))
            .ok_or_else(|| signin::invalid("Codebuff did not say which account signed in."))?
            .to_string();
        let label = value::text(user, "/email")
            .or_else(|| value::text(user, "/name"))
            .map(str::to_string);
        Ok(SignedIn {
            identity,
            label,
            document: json!({ "apiKey": token }),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use uc_core::ErrorCategory;

    #[tokio::test(start_paused = true)]
    async fn a_browser_sign_in_saves_the_cli_session_codebuff_creates() {
        let http = Scripted::new()
            .on(
                "POST",
                LOGIN_CODE,
                200,
                r#"{"fingerprintId":"x","fingerprintHash":"hash-1",
                    "loginUrl":"https://www.codebuff.com/login?auth_code=abc",
                    "expiresAt":"2026-09-27T11:00:00.000Z"}"#,
            )
            .on("GET", LOGIN_STATUS, 401, r#"{"error":"not yet"}"#)
            .on(
                "GET",
                LOGIN_STATUS,
                200,
                r#"{"user":{"id":"u-1","name":"Minh","email":"me@example.com","authToken":"cb-token"}}"#,
            );
        let context = StartContext {
            http: http.shared(),
            language: uc_core::loopback::LoginLanguage::English,
            product: NAME,
        };
        let pending = Codebuff.start(Method::GitHub, &context).await.unwrap();
        assert_eq!(pending.url, "https://www.codebuff.com/login?auth_code=abc");
        let account = pending.finish.await.result.unwrap();
        assert_eq!(account.identity, "u-1");
        assert_eq!(account.label.as_deref(), Some("me@example.com"));
        assert_eq!(account.document, json!({"apiKey": "cb-token"}));
        let requests = http.requests();
        let fingerprint: Value =
            serde_json::from_slice(requests[0].body.as_deref().unwrap()).unwrap();
        let fingerprint = fingerprint["fingerprintId"].as_str().unwrap().to_string();
        assert!(fingerprint.starts_with("quota-control-"));
        let status = url::Url::parse(&requests[1].url).unwrap();
        let query: std::collections::HashMap<_, _> = status.query_pairs().into_owned().collect();
        assert_eq!(query["fingerprintId"], fingerprint);
        assert_eq!(query["fingerprintHash"], "hash-1");
        assert_eq!(query["expiresAt"], "2026-09-27T11:00:00.000Z");
    }

    #[tokio::test]
    async fn a_login_page_off_codebuff_is_not_opened() {
        let http = Scripted::new().on(
            "POST",
            LOGIN_CODE,
            200,
            r#"{"fingerprintHash":"h","loginUrl":"https://example.com/login","expiresAt":"x"}"#,
        );
        let context = StartContext {
            http: http.shared(),
            language: uc_core::loopback::LoginLanguage::English,
            product: NAME,
        };
        let error = Codebuff
            .start(Method::GitHub, &context)
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.message,
            "Codebuff did not start a sign-in. Try again later."
        );
    }

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
        assert_eq!(found[0].label, None);
        std::fs::write(
            path.join("credentials.json"),
            r#"{"default":{"id":"u-1","email":"me@example.com","name":null,"authToken":"fake"}}"#,
        )
        .unwrap();
        let found = Codebuff.discover(&roots);
        assert_eq!(found[0].label.as_deref(), Some("me@example.com"));
    }

    #[tokio::test]
    async fn the_subscription_names_the_plan_the_account_and_the_billing_period_end() {
        let body = r#"{"usage":200,"quota":1000}"#;
        let subscription = r#"{"subscription":{"displayName":"Pro","status":"active",
            "billingPeriodEnd":"2026-10-15T00:00:00Z"},"email":"me@example.com",
            "rateLimit":{"weeklyUsed":5,"weeklyLimit":50}}"#;
        let http = Scripted::new().on("POST", URL, 200, body).on(
            "GET",
            "https://www.codebuff.com/api/user/subscription",
            200,
            subscription,
        );
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        let reading = Codebuff.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(reading.account.as_deref(), Some("me@example.com"));
        assert_eq!(
            reading.plan_term,
            Some(PlanTerm::Stated {
                ends_at: Utc.with_ymd_and_hms(2026, 10, 15, 0, 0, 0).unwrap(),
                checked_at: None,
            })
        );
        assert_eq!(reading.lines.len(), 2);
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
