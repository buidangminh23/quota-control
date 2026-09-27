//! Raycast: the Raycast AI credits used this month and the credits left, with the plan named after
//! the account's subscription tier.
//!
//! Nothing is read from disk or from a browser on Windows, macOS or Linux: the card takes the value
//! of the `__raycast_session` cookie pasted into Quota Control, and optionally a CSRF token. A
//! refresh sends one request, `GET https://www.raycast.com/frontend_api/current_user/ai_credits`,
//! whose `Cookie` header carries the session (and the CSRF token as `csrf_token` when one is
//! saved), with Raycast's site as its Origin and its settings page as its Referer. It reads
//! `remaining_balance_credits`, `total_balance_credits`, the `next_credits_at` reset and
//! `funding_subscription.tier`.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, MetricKind, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Raycast;

const NAME: &str = "Raycast";
const URL: &str = "https://www.raycast.com/frontend_api/current_user/ai_credits";

#[async_trait]
impl Service for Raycast {
    fn id(&self) -> &'static str {
        "raycast"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn key_label(&self) -> &'static str {
        "Session cookie (__raycast_session)"
    }

    fn key_format(&self) -> KeyFormat {
        KeyFormat::Cookie
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "https://www.raycast.com/settings",
            fields: &[("csrfToken", "CSRF token (optional)")],
        })
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
                Some(lines::MONTH_MS),
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
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Raycast session cookie is missing."))?;
        let body = http::json(
            context.http,
            HttpRequest::get(URL)
                .header(
                    "Cookie",
                    format!(
                        "__raycast_session={key}{}",
                        context
                            .secret
                            .str("/csrfToken")
                            .map(|token| format!("; csrf_token={v}", v = token))
                            .unwrap_or_default()
                    ),
                )
                .header("Accept", "application/json")
                .header("Origin", "https://www.raycast.com")
                .header("Referer", "https://www.raycast.com/settings"),
            NAME,
        )
        .await?;
        parse(&body)
    }
}

fn parse(body: &Value) -> Result<Reading, SimpleProviderError> {
    let left = value::number(body, "/remaining_balance_credits");
    let total = value::number(body, "/total_balance_credits");
    if left.is_none() && total.is_none()
        || left.is_some_and(|credits| credits < 0.0)
        || total.is_some_and(|credits| credits < 0.0)
    {
        return Err(http::decoding(NAME));
    }
    let mut rows = Vec::new();
    if let (Some(left), Some(total)) = (left, total) {
        rows.push(lines::count(
            "Credits",
            (total - left).max(0.0),
            total,
            "credits",
            value::time(body, "/next_credits_at"),
            Some(lines::MONTH_MS),
        ));
    }
    if let Some(left) = left {
        rows.push(lines::count_value("Balance", left, "credits"));
    }
    if rows.is_empty() {
        return Err(http::decoding(NAME));
    }
    Ok(Reading::new(
        value::text(body, "/funding_subscription/tier").and_then(lines::plan_name),
        rows,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    #[tokio::test]
    async fn credits_balance_and_plan_come_from_one_get_with_the_session_cookie() {
        let body = r###"{"remaining_balance_credits":800,"total_balance_credits":1000,"next_credits_at":"2026-10-01T00:00:00Z","funding_subscription":{"tier":"pro"}}"###;
        let http = Scripted::new().on("GET", URL, 200, body);
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey":"test"}), now);
        let reading = Raycast.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::count(
                        "Credits",
                        200.0,
                        1000.0,
                        "credits",
                        Some(Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()),
                        Some(lines::MONTH_MS)
                    ),
                    lines::count_value("Balance", 800.0, "credits")
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, URL);
        assert_eq!(
            header(&requests[0], "Cookie"),
            Some("__raycast_session=test")
        );
        assert_eq!(requests[0].body, None);
    }

    #[tokio::test]
    async fn http_errors_and_unreadable_credits_keep_their_categories_without_echoing_the_body() {
        for (status, body, category) in [
            (401, "secret", ErrorCategory::AuthExpired),
            (403, "secret", ErrorCategory::AuthExpired),
            (429, "", ErrorCategory::RateLimited),
            (500, "", ErrorCategory::Http5xx),
            (200, "{}", ErrorCategory::Decoding),
            (
                200,
                r#"{"total_balance_credits":100}"#,
                ErrorCategory::Decoding,
            ),
            (200, "bad json", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            let error = Raycast.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            Raycast.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
