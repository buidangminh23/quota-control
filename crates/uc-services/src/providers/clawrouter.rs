//! ClawRouter: the usage ledger of a ClawRouter policy key, with its spend against the monthly
//! budget and its cost, requests and tokens, in total and for each provider.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `CLAWROUTER_API_KEY` environment variable or from a key saved in Quota Control, with an optional
//! server address that must use HTTPS (`https://clawrouter.openclaw.ai` when none is given). A
//! refresh sends one request, `GET <server>/v1/usage` with the key as a bearer token. The ledger
//! counts money in micro-dollars. The Monthly Budget meter shows when the budget has both a spend
//! and a limit, and resets at midnight UTC on the first day of the month after the one its window
//! key names; the provider rows follow the totals, the costliest first.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, MetricKind, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{
    endpoint::{self, Policy},
    http, lines,
};

pub(crate) struct ClawRouter;

const NAME: &str = "ClawRouter";

#[async_trait]
impl Service for ClawRouter {
    fn id(&self) -> &'static str {
        "clawrouter"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["CLAWROUTER_API_KEY"],
            url: "https://clawrouter.openclaw.ai",
            fields: &[("baseUrl", "Server address")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let mut rows = vec![
            WidgetDescriptor::bounded_dollars(
                format!("{}.budget", provider.id),
                provider,
                "Monthly Budget",
                None,
                100.0,
                None,
                None,
            )
            .exporting_progress("budget", "usd"),
        ];
        for (id, title, kind, unit) in [
            ("cost", "Actual Cost", MetricKind::Dollars, None),
            ("requests", "Requests", MetricKind::Count, Some("requests")),
            ("tokens", "Tokens", MetricKind::Count, Some("tokens")),
        ] {
            rows.push(WidgetDescriptor::values(
                format!("{}.{id}", provider.id),
                provider,
                title,
                None,
                Some(kind),
                unit,
                true,
                None,
                false,
            ));
        }
        rows
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The ClawRouter API key is missing."))?;
        let mut base = endpoint::base_url(
            context.secret.str("/baseUrl"),
            Some("https://clawrouter.openclaw.ai"),
            Policy::Https,
            NAME,
        )?;
        if !base.ends_with("/v1") {
            base.push_str("/v1");
        }
        let body = http::json(
            context.http,
            HttpRequest::get(format!("{base}/usage")).bearer(key),
            NAME,
        )
        .await?;
        let budget = &body["budget"];
        let summary = &body["usage"]["summary"];
        let providers = body["usage"]["providers"]
            .as_array()
            .ok_or_else(|| http::decoding(NAME))?;
        if !budget["configured"].is_boolean() || !budget["ledger"].is_string() {
            return Err(http::decoding(NAME));
        }
        let requests = integer(summary, "requestCount")?;
        let tokens = integer(summary, "totalTokens")?;
        let cost = integer(summary, "actualCostMicros")? / 1e6;
        for field in ["successCount", "errorCount", "inputTokens", "outputTokens"] {
            integer(summary, field)?;
        }
        let optional = |field: &str| -> Result<Option<f64>, SimpleProviderError> {
            if budget[field].is_null() {
                Ok(None)
            } else {
                integer(budget, field).map(|micros| Some(micros / 1e6))
            }
        };
        let spent = optional("spentMicros")?;
        let limit = optional("limitMicros")?;
        optional("remainingMicros")?;
        let reset = budget["windowKey"]
            .as_str()
            .and_then(|window| window.get(window.len().checked_sub(7)?..))
            .and_then(|month| {
                chrono::NaiveDate::parse_from_str(&format!("{s}-01", s = month), "%Y-%m-%d").ok()
            })
            .and_then(|first| first.checked_add_months(chrono::Months::new(1)))
            .and_then(|next| next.and_hms_opt(0, 0, 0))
            .map(|midnight| midnight.and_utc());
        let mut rows = Vec::new();
        if let (Some(spent), Some(limit)) = (spent, limit) {
            rows.push(lines::dollars(
                "Monthly Budget",
                spent,
                limit,
                reset,
                Some(lines::MONTH_MS),
            ));
        }
        rows.push(lines::dollar_value("Actual Cost", cost));
        rows.push(lines::count_value("Requests", requests, "requests"));
        rows.push(lines::count_value("Tokens", tokens, "tokens"));
        let mut details = Vec::new();
        for item in providers {
            let name = item["provider"]
                .as_str()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or("Unknown");
            if !item["provider"].is_string() {
                return Err(http::decoding(NAME));
            }
            let requests = integer(item, "requestCount")?;
            let cost = integer(item, "actualCostMicros")? / 1e6;
            let tokens = integer(item, "totalTokens")?;
            integer(item, "successCount")?;
            integer(item, "errorCount")?;
            details.push((name, cost, requests, tokens));
        }
        details.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| right.2.total_cmp(&left.2))
                .then_with(|| left.0.cmp(right.0))
        });
        for (name, cost, requests, tokens) in details {
            rows.push(lines::dollar_value(&format!("{name} Cost"), cost));
            rows.push(lines::count_value(
                &format!("{name} Requests"),
                requests,
                "requests",
            ));
            rows.push(lines::count_value(
                &format!("{name} Tokens"),
                tokens,
                "tokens",
            ));
        }
        Ok(Reading::new(None, rows))
    }
}

/// The whole, non-negative number in `field` of `object`, as a float.
fn integer(object: &Value, field: &str) -> Result<f64, SimpleProviderError> {
    object[field]
        .as_i64()
        .filter(|number| *number >= 0)
        .map(|number| number as f64)
        .ok_or_else(|| http::decoding(NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use serde_json::json;
    use uc_core::ErrorCategory;

    fn fixture() -> Value {
        json!({"budget":{"configured":true,"ledger":"billing","limitMicros":10_000_000,"spentMicros":2_000_000,"remainingMicros":8_000_000,"windowKey":"month:2026-12"},"usage":{"summary":{"requestCount":2,"successCount":1,"errorCount":1,"inputTokens":10,"outputTokens":5,"totalTokens":15,"actualCostMicros":2_000_000},"providers":[{"provider":"model-a","requestCount":2,"successCount":1,"errorCount":1,"totalTokens":15,"actualCostMicros":2_000_000}]}})
    }

    #[tokio::test]
    async fn micro_dollars_show_as_dollars_with_the_next_month_reset_and_rows_per_provider() {
        let http = Scripted::new().on(
            "GET",
            "https://clawrouter.openclaw.ai/v1/usage",
            200,
            &fixture().to_string(),
        );
        let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
        let reading = ClawRouter.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines.len(), 7);
        let reset = chrono::NaiveDate::from_ymd_opt(2027, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        assert_eq!(
            reading.lines[0],
            lines::dollars(
                "Monthly Budget",
                2.0,
                10.0,
                Some(reset),
                Some(lines::MONTH_MS)
            )
        );
        assert_eq!(reading.lines[4], lines::dollar_value("model-a Cost", 2.0));
        assert_eq!(
            header(&http.requests()[0], "authorization"),
            Some("Bearer fixture")
        );
    }

    #[tokio::test]
    async fn failures_keep_their_categories_and_a_plain_http_address_is_refused_unsent() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{}", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", "https://clawrouter.openclaw.ai", status, body);
            let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
            assert_eq!(
                ClawRouter
                    .fetch(&scope.context())
                    .await
                    .unwrap_err()
                    .category,
                category
            );
        }
        let http = Scripted::new();
        let scope = context_at(
            &http,
            json!({"apiKey":"fixture","baseUrl":"http://127.0.0.1"}),
            chrono::Utc::now(),
        );
        assert!(ClawRouter.fetch(&scope.context()).await.is_err());
        assert!(http.requests().is_empty());
    }
}
