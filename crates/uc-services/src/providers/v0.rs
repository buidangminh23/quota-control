//! v0: how much of a v0 account's credits and API requests is used, and its on-demand credit
//! balance.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the `V0_API_KEY`
//! environment variable or from a key saved in Accounts, with an optional scope beside it. A
//! refresh sends two `GET`s with the key as a bearer token, each with `?scope=<scope>` when a scope
//! is saved:
//! - `https://api.v0.dev/v1/user/billing` for the credits, read from a `token` billing type's
//!   balance and billing cycle or from a `legacy` one's limit and reset, and for the on-demand
//!   balance;
//! - `https://api.v0.dev/v1/rate-limits` for the request limit, what remains of it and when it
//!   resets.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct V0;

const NAME: &str = "v0";
const URL: &str = "https://api.v0.dev/v1/user/billing";

#[async_trait]
impl Service for V0 {
    fn id(&self) -> &'static str {
        "v0"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["V0_API_KEY"],
            url: "https://v0.dev/chat/settings/keys",
            fields: &[("scope", "Scope (optional)")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let mut descriptors: Vec<_> = [("credits", "Credits"), ("requests", "Requests")]
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
            .collect();
        descriptors.push(WidgetDescriptor::values(
            format!("{}.onDemand", provider.id),
            provider,
            "On-Demand",
            None,
            Some(uc_core::MetricKind::Count),
            Some("credits"),
            false,
            None,
            false,
        ));
        descriptors
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The v0 API key is missing."))?;
        let query = context
            .secret
            .str("/scope")
            .map(|scope| {
                format!(
                    "?scope={}",
                    url::form_urlencoded::byte_serialize(scope.as_bytes()).collect::<String>()
                )
            })
            .unwrap_or_default();
        let body = http::json(
            context.http,
            HttpRequest::get(format!("{URL}{query}")).bearer(key),
            NAME,
        )
        .await?;
        let mut reading = parse(&body)?;
        let rate_limits = http::json(
            context.http,
            HttpRequest::get(format!("https://api.v0.dev/v1/rate-limits{query}")).bearer(key),
            NAME,
        )
        .await?;
        let limit = value::number(&rate_limits, "/limit").ok_or_else(|| http::decoding(NAME))?;
        if limit < 0.0 {
            return Err(http::decoding(NAME));
        }
        if let Some(left) = value::number(&rate_limits, "/remaining") {
            reading.lines.push(lines::percent(
                "Requests",
                if limit > 0.0 {
                    (limit - left) / limit * 100.0
                } else {
                    100.0
                },
                value::time(&rate_limits, "/reset"),
                None,
            ));
        } else {
            reading
                .lines
                .push(lines::count_value("Requests", limit, "requests"));
        }
        if let Some(balance) = value::number(&body, "/data/onDemand/balance") {
            reading
                .lines
                .push(lines::count_value("On-Demand", balance, "credits"));
        }
        Ok(reading)
    }
}

/// The plan and the Credits line of the billing answer: the share used when the answer says what
/// remains, else the credit limit as a count. A `token` billing type is read from its balance and
/// billing cycle, a `legacy` one from its limit and reset; any other type cannot be read.
fn parse(body: &Value) -> Result<Reading, SimpleProviderError> {
    let mut credits = Vec::new();
    let (limit, left, reset) = match value::text(body, "/billingType") {
        Some("token") => (
            value::number(body, "/data/balance/total"),
            value::number(body, "/data/balance/remaining"),
            value::time(body, "/data/billingCycle/end"),
        ),
        Some("legacy") => (
            value::number(body, "/data/limit"),
            value::number(body, "/data/remaining"),
            value::time(body, "/data/reset"),
        ),
        _ => return Err(http::decoding(NAME)),
    };
    let limit = limit.ok_or_else(|| http::decoding(NAME))?;
    if limit < 0.0 {
        return Err(http::decoding(NAME));
    }
    if let Some(left) = left {
        credits.push(lines::percent(
            "Credits",
            if limit > 0.0 {
                (limit - left) / limit * 100.0
            } else {
                100.0
            },
            reset,
            None,
        ));
    } else {
        credits.push(lines::count_value("Credits", limit, "credits"));
    }
    Ok(Reading::new(Some("API".into()), credits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const TOKEN_BILLING: &str = r###"{"billingType":"token","data":{"balance":{"total":100,"remaining":75},"onDemand":{"balance":12.5},"billingCycle":{"end":1790812800000}}}"###;

    #[tokio::test]
    async fn reads_credits_requests_and_the_on_demand_balance_with_the_key_as_a_bearer() {
        let http = Scripted::new().on("GET", URL, 200, TOKEN_BILLING);
        let http = http.on(
            "GET",
            "https://api.v0.dev/v1/rate-limits",
            200,
            r#"{"limit":50,"remaining":40,"reset":1790510400}"#,
        );
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey":"test"}), now);
        let reading = V0.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("API".into()),
                vec![
                    lines::percent(
                        "Credits",
                        25.0,
                        value::as_time(&json!(1790812800000_i64)),
                        None
                    ),
                    lines::percent("Requests", 20.0, value::as_time(&json!(1790510400)), None),
                    lines::count_value("On-Demand", 12.5, "credits")
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url, URL);
        assert_eq!(header(&requests[0], "Authorization"), Some("Bearer test"));
        assert_eq!(requests[0].body, None);
    }

    #[tokio::test]
    async fn failed_or_unreadable_billing_answers_keep_their_categories_without_echoing_them() {
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
            let error = V0.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            V0.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
