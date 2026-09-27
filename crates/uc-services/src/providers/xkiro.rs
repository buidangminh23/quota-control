//! xKiro: the free tokens an xKiro API key has used today against its daily limit, the free tokens
//! left, and the plan's name.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the `XKIRO_API_KEY`
//! environment variable or from a key saved in Quota Control. A refresh sends one read-only
//! request, `GET https://api.xkiro.com/v1/usage` with the key as a bearer token. The answer must be
//! a `usage` object with a `free_tokens` object whose counts are non-negative whole numbers; the
//! day's window ends at the next UTC midnight, and an account whose `plan` is null is on pay as
//! you go.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, MetricKind, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct XKiro;

const NAME: &str = "xKiro";
const URL: &str = "https://api.xkiro.com/v1/usage";

#[async_trait]
impl Service for XKiro {
    fn id(&self) -> &'static str {
        "xkiro"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["XKIRO_API_KEY"],
            url: "https://xkiro.com",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::bounded_count(
                format!("{}.daily", provider.id),
                provider,
                "Daily",
                None,
                0.0,
                "tokens",
                Some(lines::DAY_MS),
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
            .ok_or_else(|| http::invalid("The xKiro API key is missing."))?;
        let body = http::json(context.http, HttpRequest::get(URL).bearer(key), NAME).await?;
        parse(&body, context)
    }
}

/// The usage answer as the Daily and Balance rows and the plan's name.
fn parse(body: &Value, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
    if value::text(body, "/object") != Some("usage")
        || !body.get("free_tokens").is_some_and(Value::is_object)
    {
        return Err(http::decoding(NAME));
    }
    let used = value::number(body, "/free_tokens/used_today");
    let limit = value::number(body, "/free_tokens/limit_per_day");
    let left = value::number(body, "/free_tokens/remaining");
    for count in [used, limit, left].into_iter().flatten() {
        if count < 0.0 || count.fract() != 0.0 || count > 9007199254740991.0 {
            return Err(http::decoding(NAME));
        }
    }
    let reset = context
        .now
        .date_naive()
        .succ_opt()
        .and_then(|tomorrow| tomorrow.and_hms_opt(0, 0, 0))
        .map(|midnight| midnight.and_utc());
    let mut rows = Vec::new();
    if let (Some(used), Some(limit)) = (used, limit) {
        rows.push(lines::count(
            "Daily",
            used,
            limit,
            "tokens",
            reset,
            Some(lines::DAY_MS),
        ));
    } else if let Some(used) = used {
        rows.push(lines::count_value("Daily", used, "tokens"));
    }
    if let Some(left) = left {
        rows.push(lines::count_value("Balance", left, "tokens"));
    }
    if rows.is_empty() {
        return Err(http::decoding(NAME));
    }
    Ok(Reading::new(
        if body.get("plan") == Some(&Value::Null) {
            Some("Pay As You Go".into())
        } else {
            value::text(body, "/plan").and_then(lines::plan_name)
        },
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
    async fn reads_todays_free_tokens_the_tokens_left_and_the_plan_with_the_key_as_a_bearer() {
        let body = r###"{"object":"usage","free_tokens":{"used_today":250,"limit_per_day":1000,"remaining":750},"plan":"pro"}"###;
        let http = Scripted::new().on("GET", URL, 200, body);
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey":"test"}), now);
        let reading = XKiro.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::count(
                        "Daily",
                        250.0,
                        1000.0,
                        "tokens",
                        Some(Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap()),
                        Some(lines::DAY_MS)
                    ),
                    lines::count_value("Balance", 750.0, "tokens")
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, URL);
        assert_eq!(header(&requests[0], "Authorization"), Some("Bearer test"));
        assert_eq!(requests[0].body, None);
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
            let http = Scripted::new().on("GET", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            let error = XKiro.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            XKiro.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
