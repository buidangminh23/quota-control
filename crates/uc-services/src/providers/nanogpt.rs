//! NanoGPT: the prepaid balance of a NanoGPT account and the counters of its subscription.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `NANOGPT_API_KEY` environment variable or from a key saved in Accounts, and every request
//! carries it in the `x-api-key` header. A refresh sends two requests. The first,
//! `POST https://api.nano-gpt.com/api/check-balance`, is a read-only balance query that gives the
//! dollar Balance and the Nano balance. The second,
//! `GET https://api.nano-gpt.com/api/subscription/v1/usage`, gives the subscription's state, shown
//! as the plan, and its daily and weekly input tokens, daily images and trial tokens, each as a
//! meter against its limit when NanoGPT reports one and as a plain count otherwise, and the end of
//! the subscription's billing period (`period.currentPeriodEnd`). When that
//! endpoint answers 404 the balances are shown with a warning that no subscription usage was
//! provided, and counters NanoGPT marks as degraded are left out with a warning. The endpoints are
//! documented at https://docs.nano-gpt.com/api-reference/endpoint/check-balance.md and
//! https://docs.nano-gpt.com/api-reference/endpoint/subscription-usage.md.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, PlanTerm, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct NanoGpt;

const BALANCE_URL: &str = "https://api.nano-gpt.com/api/check-balance";
const USAGE_URL: &str = "https://api.nano-gpt.com/api/subscription/v1/usage";

#[async_trait]
impl Service for NanoGpt {
    fn id(&self) -> &'static str {
        "nanogpt"
    }

    fn name(&self) -> &'static str {
        "NanoGPT"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["NANOGPT_API_KEY"],
            url: "https://nano-gpt.com/api",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let mut descriptors = vec![
            WidgetDescriptor::dollar_balance(
                format!("{}.balance", provider.id),
                provider,
                "Balance",
                None,
                "left",
            ),
            WidgetDescriptor::values(
                format!("{}.nano", provider.id),
                provider,
                "Nano",
                None,
                Some(uc_core::MetricKind::Count),
                None,
                false,
                None,
                false,
            ),
        ];
        for (s, title, period) in [
            ("daily", "Daily Input Tokens", lines::DAY_MS),
            ("weekly", "Weekly Input Tokens", lines::WEEK_MS),
        ] {
            descriptors.push(WidgetDescriptor::bounded_count(
                format!("{}.{s}", provider.id),
                provider,
                title,
                None,
                0.0,
                "tokens",
                Some(period),
            ));
        }
        descriptors
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The NanoGPT API key is missing."))?;
        let balance = http::json(
            context.http,
            HttpRequest::post(BALANCE_URL).header("x-api-key", key),
            "NanoGPT",
        )
        .await?;
        let usd =
            value::number(&balance, "/usd_balance").ok_or_else(|| http::decoding("NanoGPT"))?;
        let mut rows = vec![lines::dollar_value("Balance", usd)];
        if let Some(nano) = value::number(&balance, "/nano_balance") {
            rows.push(lines::count_value("Nano", nano, "Nano"));
        }
        let response = http::send(
            context.http,
            HttpRequest::get(USAGE_URL).header("x-api-key", key),
            "NanoGPT",
        )
        .await?;
        if response.status == 404 {
            return Ok(Reading::new(None, rows)
                .with_warning(Some("NanoGPT did not provide subscription usage.".into())));
        }
        if !response.is_success() {
            return Err(http::status_error(&response, "NanoGPT"));
        }
        let usage: Value = http::parse(&response, "NanoGPT")?;
        if usage.get("active").and_then(Value::as_bool).is_none()
            && ![
                "dailyInputTokens",
                "weeklyInputTokens",
                "dailyImages",
                "tokens",
            ]
            .iter()
            .any(|field| usage.get(*field).is_some())
        {
            return Err(http::decoding("NanoGPT"));
        }
        let mut degraded = false;
        for (field, label, unit, period) in [
            (
                "dailyInputTokens",
                "Daily Input Tokens",
                "tokens",
                Some(lines::DAY_MS),
            ),
            (
                "weeklyInputTokens",
                "Weekly Input Tokens",
                "tokens",
                Some(lines::WEEK_MS),
            ),
            ("dailyImages", "Daily Images", "images", Some(lines::DAY_MS)),
            ("tokens", "Trial Tokens", "tokens", None),
        ] {
            let Some(bucket) = usage.get(field).filter(|entry| entry.is_object()) else {
                continue;
            };
            if value::flag(bucket, "/degraded") == Some(true) {
                degraded = true;
                continue;
            }
            let Some(used) = value::number(bucket, "/used").filter(|used| *used >= 0.0) else {
                continue;
            };
            let limit = if field == "tokens" {
                value::number(&usage, "/tokenLimits/total")
            } else {
                value::number(&usage, &format!("/limits/{field}"))
            };
            let row = if let Some(limit) = limit.filter(|limit| *limit > 0.0) {
                lines::count(
                    label,
                    used,
                    limit,
                    unit,
                    value::time(bucket, "/resetAt"),
                    period,
                )
            } else {
                lines::count_value(label, used, unit)
            };
            rows.push(row);
        }
        let term =
            value::time(&usage, "/period/currentPeriodEnd").map(|ends_at| PlanTerm::Stated {
                ends_at,
                checked_at: None,
            });
        Ok(
            Reading::new(value::text(&usage, "/state").map(str::to_owned), rows)
                .with_plan_term(term)
                .with_warning(degraded.then(|| {
                    "Some NanoGPT subscription counters are temporarily unavailable.".into()
                })),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Utc;
    use serde_json::json;
    use uc_core::ErrorCategory;

    const SUBSCRIPTION: &str = r#"{"active":true,"state":"active","limits":{"dailyInputTokens":100,"weeklyInputTokens":500,"dailyImages":null},"dailyInputTokens":{"used":120,"remaining":0,"percentUsed":1.2,"resetAt":1790812800000},"weeklyInputTokens":{"used":200,"remaining":300,"percentUsed":0.4,"resetAt":1790812800000},"dailyImages":null,"period":{"currentPeriodEnd":"2026-10-15T00:00:00.000Z"}}"#;
    const DEGRADED: &str =
        r#"{"dailyInputTokens":{"degraded":true,"used":null},"limits":{"dailyInputTokens":100}}"#;

    #[tokio::test]
    async fn reads_balances_and_subscription_counters_with_the_key_in_x_api_key() {
        let http = Scripted::new()
            .on(
                "POST",
                BALANCE_URL,
                200,
                r#"{"usd_balance":"12.50","nano_balance":"0.4"}"#,
            )
            .on("GET", USAGE_URL, 200, SUBSCRIPTION);
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        let reading = NanoGpt.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("active"));
        assert_eq!(
            reading.plan_term,
            value::as_time(&json!("2026-10-15T00:00:00Z")).map(|ends_at| PlanTerm::Stated {
                ends_at,
                checked_at: None,
            })
        );
        assert_eq!(
            reading.lines,
            vec![
                lines::dollar_value("Balance", 12.5),
                lines::count_value("Nano", 0.4, "Nano"),
                lines::count(
                    "Daily Input Tokens",
                    120.0,
                    100.0,
                    "tokens",
                    value::as_time(&json!(1790812800000i64)),
                    Some(lines::DAY_MS)
                ),
                lines::count(
                    "Weekly Input Tokens",
                    200.0,
                    500.0,
                    "tokens",
                    value::as_time(&json!(1790812800000i64)),
                    Some(lines::WEEK_MS)
                )
            ]
        );
        assert_eq!(http.requests().len(), 2);
        assert_eq!(header(&http.requests()[0], "x-api-key"), Some("test"));
    }

    #[tokio::test]
    async fn a_degraded_counter_is_left_out_with_a_warning() {
        let http = Scripted::new()
            .on("POST", BALANCE_URL, 200, r#"{"usd_balance":0}"#)
            .on("GET", USAGE_URL, 200, DEGRADED);
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        let reading = NanoGpt.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines.len(), 1);
        assert!(reading.warning.is_some());
    }

    #[tokio::test]
    async fn refused_rate_limited_and_unreadable_balances_keep_their_categories() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (200, ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("POST", BALANCE_URL, status, "{}");
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            assert_eq!(
                NanoGpt.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }
}
