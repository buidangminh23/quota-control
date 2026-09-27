//! T3 Chat: the four-hour and monthly usage of a T3 Chat subscription, and the plan's name.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the user pastes the `session` cookie of a
//! signed-in T3 Chat browser session into Quota Control. A refresh sends one request, the batched
//! tRPC query `getCustomerData` (`GET https://t3.chat/api/trpc/getCustomerData?batch=1&input=…`,
//! whose input leaves `sessionId` undefined), carrying the cookie as `session=<value>`, the site's
//! Origin and Referer (`https://t3.chat` and its customization settings page) and the tRPC headers
//! `trpc-accept: application/jsonl`, `x-trpc-source: web-client` and `x-trpc-batch: true`.
//!
//! The answer is read line by line as JSON, and the first line holding the usage object gives the
//! Session meter (the four-hour window) and the Monthly meter (the month, or else the billing
//! period). The plan is the subscription's product name, or else the answer's `subTier`.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct T3Chat;

const NAME: &str = "T3 Chat";
const URL: &str = "https://t3.chat/api/trpc/getCustomerData?batch=1&input=%7B%220%22%3A%7B%22json%22%3A%7B%22sessionId%22%3Anull%7D%2C%22meta%22%3A%7B%22values%22%3A%7B%22sessionId%22%3A%5B%22undefined%22%5D%7D%7D%7D%7D";

#[async_trait]
impl Service for T3Chat {
    fn id(&self) -> &'static str {
        "t3chat"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn key_label(&self) -> &'static str {
        "Session cookie (session)"
    }

    fn key_format(&self) -> KeyFormat {
        KeyFormat::Cookie
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "https://t3.chat/settings/customization",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [("session", "Session"), ("monthly", "Monthly")]
            .into_iter()
            .map(|(id, title)| {
                WidgetDescriptor::percent(
                    format!("{}.{id}", provider.id),
                    provider,
                    title,
                    None,
                    None,
                )
                .exporting_progress(id, "percent")
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The T3 Chat session cookie is missing."))?;
        let response = http::send(
            context.http,
            HttpRequest::get(URL)
                .header("Cookie", format!("session={key}"))
                .header("Origin", "https://t3.chat")
                .header("Referer", "https://t3.chat/settings/customization")
                .header("trpc-accept", "application/jsonl")
                .header("x-trpc-source", "web-client")
                .header("x-trpc-batch", "true"),
            NAME,
        )
        .await?;
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        for line in String::from_utf8_lossy(&response.body).lines() {
            if let Ok(body) = serde_json::from_str::<Value>(line)
                && find(&body, 0).is_some()
            {
                return parse(&body);
            }
        }
        Err(http::decoding(NAME))
    }
}

/// The Session and Monthly meters and the plan, from the usage object inside an answer line.
fn parse(body: &Value) -> Result<Reading, SimpleProviderError> {
    let usage = find(body, 0).ok_or_else(|| http::decoding(NAME))?;
    let session = value::number(usage, "/usageFourHourPercentage");
    let month = value::number(usage, "/usageMonthPercentage")
        .or_else(|| value::number(usage, "/usagePeriodPercentage"));
    if session.is_none() && month.is_none() {
        return Err(http::decoding(NAME));
    }
    let mut rows = Vec::new();
    if let Some(used) = session {
        rows.push(lines::percent(
            "Session",
            used,
            value::time(usage, "/usageFourHourNextResetAt")
                .or_else(|| value::time(usage, "/usageWindowNextResetAt")),
            Some(4 * lines::HOUR_MS),
        ));
    }
    if let Some(used) = month {
        rows.push(lines::percent(
            "Monthly",
            used,
            value::time(usage, "/subscription/currentPeriodEnd"),
            Some(lines::MONTH_MS),
        ));
    }
    Ok(Reading::new(
        value::text(usage, "/subscription/productName")
            .or_else(|| value::text(usage, "/subTier"))
            .and_then(lines::plan_name),
        rows,
    ))
}

/// The object in `node`, searched at most 32 levels deep, that carries a usage percentage.
fn find(node: &Value, depth: u8) -> Option<&Value> {
    if depth > 32 {
        return None;
    }
    if node.get("usageFourHourPercentage").is_some()
        || node.get("usageMonthPercentage").is_some()
        || node.get("usagePeriodPercentage").is_some()
    {
        return Some(node);
    }
    match node {
        Value::Array(items) => items.iter().find_map(|item| find(item, depth + 1)),
        Value::Object(items) => items.values().find_map(|item| find(item, depth + 1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const CUSTOMER_DATA: &str = r###"[{"result":{"data":{"json":{"usageFourHourPercentage":25,"usageMonthPercentage":50,"usageFourHourNextResetAt":1790510400,"subscription":{"productName":"pro","currentPeriodEnd":1790812800}}}}}]"###;

    #[tokio::test]
    async fn one_cookie_request_shows_the_four_hour_and_monthly_meters_and_the_plan() {
        let http = Scripted::new().on("GET", URL, 200, CUSTOMER_DATA);
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey":"test"}), now);
        let reading = T3Chat.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::percent(
                        "Session",
                        25.0,
                        value::as_time(&json!(1790510400)),
                        Some(4 * lines::HOUR_MS)
                    ),
                    lines::percent(
                        "Monthly",
                        50.0,
                        value::as_time(&json!(1790812800)),
                        Some(lines::MONTH_MS)
                    )
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, URL);
        assert_eq!(header(&requests[0], "Cookie"), Some("session=test"));
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
            let error = T3Chat.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            T3Chat.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
