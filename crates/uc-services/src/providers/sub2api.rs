//! sub2api: the limits, balance and usage a sub2api gateway reports for one of its API keys.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the API key and the gateway's server
//! address are saved in Quota Control. The address must use HTTPS, or plain HTTP to this computer
//! only, and may already end with `/v1` or `/v1/usage`. A refresh sends one read-only request,
//! `GET <address>/v1/usage?days=30&timezone=UTC` with the key as a bearer token. The card shows
//! the subscription's daily, weekly and monthly dollar limits, or without a subscription the key's
//! own quota, then the balance, today's and total requests, tokens and cost, and each rate-limit
//! window with its reset time; the key quota and the balance use the unit the answer names,
//! dollars by default. A key the gateway marks invalid reads as refused.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Sub2Api;

const NAME: &str = "sub2api";

#[cfg(test)]
const URL: &str = "https://gateway.example/v1/usage?days=30&timezone=UTC";

#[async_trait]
impl Service for Sub2Api {
    fn id(&self) -> &'static str {
        "sub2api"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "https://github.com/Wei-Shaw/sub2api",
            fields: &[("baseUrl", "Server address")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [
            ("daily", "Daily"),
            ("weekly", "Weekly"),
            ("monthly", "Monthly"),
            ("keyLimit", "Key Limit"),
            ("balance", "Balance"),
            ("today", "Today"),
            ("total", "Total Usage"),
        ]
        .into_iter()
        .map(|(id, title)| {
            WidgetDescriptor::combined(
                format!("{}.{id}", provider.id),
                provider,
                title,
                None,
                false,
            )
        })
        .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The sub2api API key is missing."))?;
        let mut base = crate::support::endpoint::base_url(
            context.secret.str("/baseUrl"),
            None,
            crate::support::endpoint::Policy::HttpsOrLoopbackHttp,
            NAME,
        )?;
        if !base.ends_with("/v1") && !base.ends_with("/v1/usage") {
            base.push_str("/v1");
        }
        if !base.ends_with("/usage") {
            base.push_str("/usage");
        }
        let endpoint = format!("{base}?days=30&timezone=UTC");
        let body = http::json(context.http, HttpRequest::get(endpoint).bearer(key), NAME).await?;
        parse(&body)
    }
}

/// The rows of a usage answer; an answer that yields none of them cannot be read.
fn parse(body: &Value) -> Result<Reading, SimpleProviderError> {
    if value::flag(body, "/isValid") == Some(false) {
        return Err(http::expired(
            "sub2api rejected the API key. Check that it is active and assigned to a group.",
        ));
    }
    let mut out = Vec::new();
    for (id, label, period) in [
        ("daily", "Daily", lines::DAY_MS),
        ("weekly", "Weekly", lines::WEEK_MS),
        ("monthly", "Monthly", lines::MONTH_MS),
    ] {
        if let Some(limit) = value::number(body, &format!("/subscription/{id}_limit_usd")) {
            let used = value::number(body, &format!("/subscription/{id}_usage_usd")).unwrap_or(0.0);
            out.push(lines::dollars(label, used, limit, None, Some(period)));
        }
    }
    let unit = value::text(body, "/unit")
        .or_else(|| value::text(body, "/quota/unit"))
        .unwrap_or("USD");
    if !body.get("subscription").is_some_and(Value::is_object)
        && let (Some(used), Some(limit)) = (
            value::number(body, "/quota/used"),
            value::number(body, "/quota/limit"),
        )
    {
        out.push(if unit.eq_ignore_ascii_case("usd") {
            lines::dollars("Key Limit", used, limit, None, None)
        } else {
            lines::count("Key Limit", used, limit, unit, None, None)
        });
    }
    if let Some(balance) = value::number(body, "/balance") {
        out.push(if unit.eq_ignore_ascii_case("usd") {
            lines::dollar_value("Balance", balance)
        } else {
            lines::count_value("Balance", balance, unit)
        });
    }
    for (path, label) in [("/usage/today", "Today"), ("/usage/total", "Total Usage")] {
        if let Some(usage) = body.pointer(path) {
            let mut amounts = Vec::new();
            for (field, unit) in [("/requests", "requests"), ("/total_tokens", "tokens")] {
                if let Some(amount) = value::number(usage, field) {
                    amounts.push(uc_core::MetricValue::count(amount, unit));
                }
            }
            if let Some(cost) = value::number(usage, "/actual_cost") {
                amounts.push(uc_core::MetricValue::dollars(cost));
            }
            if !amounts.is_empty() {
                out.push(lines::values(label, amounts));
            }
        }
    }
    if let Some(rates) = body.get("rate_limits").and_then(Value::as_array) {
        for rate in rates {
            let label = value::text(rate, "/window").ok_or_else(|| http::decoding(NAME))?;
            let used = value::number(rate, "/used").ok_or_else(|| http::decoding(NAME))?;
            let limit = value::number(rate, "/limit").ok_or_else(|| http::decoding(NAME))?;
            out.push(lines::dollars(
                &format!("{label} Limit"),
                used,
                limit,
                value::time(rate, "/reset_at"),
                None,
            ));
        }
    }
    if out.is_empty() {
        return Err(http::decoding(NAME));
    }
    Ok(Reading::new(
        value::text(body, "/planName").and_then(lines::plan_name),
        out,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const USAGE: &str = r###"{"isValid":true,"planName":"pro","balance":12,"subscription":{"daily_usage_usd":2,"daily_limit_usd":10},"usage":{"today":{"requests":4,"total_tokens":100,"actual_cost":0.2}},"rate_limits":[{"window":"5h","used":1,"limit":5,"remaining":4,"reset_at":"2026-09-27T15:00:00Z"}]}"###;

    #[tokio::test]
    async fn a_subscription_key_shows_its_limits_balance_usage_and_rate_limits_from_one_read() {
        let http = Scripted::new().on("GET", URL, 200, USAGE);
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(
            &http,
            json!({"apiKey":"test","baseUrl":"https://gateway.example"}),
            now,
        );
        let reading = Sub2Api.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::dollars("Daily", 2.0, 10.0, None, Some(lines::DAY_MS)),
                    lines::dollar_value("Balance", 12.0),
                    lines::values(
                        "Today",
                        vec![
                            uc_core::MetricValue::count(4.0, "requests"),
                            uc_core::MetricValue::count(100.0, "tokens"),
                            uc_core::MetricValue::dollars(0.2)
                        ]
                    ),
                    lines::dollars(
                        "5h Limit",
                        1.0,
                        5.0,
                        Some(Utc.with_ymd_and_hms(2026, 9, 27, 15, 0, 0).unwrap()),
                        None
                    )
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
    async fn failed_reads_keep_their_category_and_never_repeat_the_answer() {
        for (status, body, category) in [
            (401, "secret", ErrorCategory::AuthExpired),
            (403, "secret", ErrorCategory::AuthExpired),
            (429, "", ErrorCategory::RateLimited),
            (500, "", ErrorCategory::Http5xx),
            (200, "{}", ErrorCategory::Decoding),
            (200, "bad json", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", URL, status, body);
            let scope = context_at(
                &http,
                json!({"apiKey":"test","baseUrl":"https://gateway.example"}),
                Utc::now(),
            );
            let error = Sub2Api.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            Sub2Api.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
