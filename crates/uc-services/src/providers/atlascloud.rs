//! Atlas Cloud: the dollar balance of an Atlas Cloud account.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `ATLASCLOUD_API_KEY` environment variable or from a key saved in Quota Control. A refresh sends
//! one request, `GET https://api.atlascloud.ai/public/v1/balance` with the key as a bearer token,
//! and shows `available.value` as the dollars left, a negative balance included. Only the balance
//! of the whole account in `usd`, written as a plain decimal, is read; any other answer is data
//! this version cannot read.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines};

pub(crate) struct AtlasCloud;

const NAME: &str = "Atlas Cloud";
const URL: &str = "https://api.atlascloud.ai/public/v1/balance";

#[async_trait]
impl Service for AtlasCloud {
    fn id(&self) -> &'static str {
        "atlascloud"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["ATLASCLOUD_API_KEY"],
            url: "https://www.atlascloud.ai/console",
            fields: &[],
        })
    }

    fn key_label(&self) -> &'static str {
        "Atlas Cloud API key"
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![WidgetDescriptor::dollar_balance(
            format!("{}.balance", provider.id),
            provider,
            "Balance",
            None,
            "left",
        )]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Atlas Cloud API key is missing."))?;
        let body = http::json(
            context.http,
            HttpRequest::get(URL)
                .bearer(key)
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        let raw = body
            .pointer("/available/value")
            .and_then(Value::as_str)
            .ok_or_else(|| http::decoding(NAME))?;
        let unsigned = raw.strip_prefix('-').unwrap_or(raw);
        let mut parts = unsigned.split('.');
        let whole = parts.next().unwrap_or_default();
        let fraction = parts.next();
        if body["object"] != "balance"
            || body["scope"] != "account"
            || body.pointer("/available/currency").and_then(Value::as_str) != Some("usd")
            || whole.is_empty()
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || fraction.is_some_and(|digits| {
                digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit())
            })
            || parts.next().is_some()
        {
            return Err(http::decoding(NAME));
        }
        let balance = raw
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .ok_or_else(|| http::decoding(NAME))?;
        Ok(Reading::new(
            None,
            vec![lines::dollar_value("Balance", balance)],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Utc;
    use serde_json::json;
    use uc_core::ErrorCategory;

    #[tokio::test]
    async fn positive_negative_and_zero_balances_are_read_with_one_bearer_request() {
        for amount in ["12.75", "-2.50", "0"] {
            let body = json!({
                "object": "balance",
                "scope": "account",
                "available": {"value": amount, "currency": "usd"}
            })
            .to_string();
            let http = Scripted::new().on("GET", URL, 200, &body);
            let scope = context_at(&http, json!({"apiKey":"fixture"}), Utc::now());
            assert_eq!(
                AtlasCloud.fetch(&scope.context()).await.unwrap(),
                Reading::new(
                    None,
                    vec![lines::dollar_value("Balance", amount.parse().unwrap())]
                )
            );
            let requests = http.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].url, URL);
            assert_eq!(
                header(&requests[0], "authorization"),
                Some("Bearer fixture")
            );
        }
    }

    #[tokio::test]
    async fn refusals_rate_limits_and_answers_outside_the_balance_contract_keep_their_category() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{", ErrorCategory::Decoding),
            (
                200,
                r#"{"object":"balance","scope":"key","available":{"value":"2","currency":"usd"}}"#,
                ErrorCategory::Decoding,
            ),
            (
                200,
                r#"{"object":"balance","scope":"account","available":{"value":"1e2","currency":"usd"}}"#,
                ErrorCategory::Decoding,
            ),
            (
                200,
                r#"{"object":"balance","scope":"account","available":{"value":"2","currency":"eur"}}"#,
                ErrorCategory::Decoding,
            ),
        ] {
            let http = Scripted::new().on("GET", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"fixture"}), Utc::now());
            assert_eq!(
                AtlasCloud
                    .fetch(&scope.context())
                    .await
                    .unwrap_err()
                    .category,
                category
            );
        }
    }
}
