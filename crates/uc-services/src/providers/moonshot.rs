//! Moonshot: the available, cash and voucher balances of a Moonshot (Kimi Open Platform) API key.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the `MOONSHOT_API_KEY`
//! or `MOONSHOT_KEY` environment variable or from a key saved in Quota Control. A refresh sends
//! `GET /v1/users/me/balance` with the key as a bearer token to the global host
//! `https://api.moonshot.ai`, and when that host refuses the key (HTTP 401 or 403) sends it again
//! to the China host `https://api.moonshot.cn`. The host that answered is remembered for 12 hours
//! and asked first until then. Balances from the global host show as dollars, and those from the
//! China host as `CNY` amounts.

use async_trait::async_trait;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines};

pub(crate) struct Moonshot;

const NAME: &str = "Moonshot";
const HOSTS: [&str; 2] = ["https://api.moonshot.ai", "https://api.moonshot.cn"];

#[async_trait]
impl Service for Moonshot {
    fn id(&self) -> &'static str {
        "moonshot"
    }

    fn name(&self) -> &'static str {
        "Moonshot / Kimi Open Platform"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["MOONSHOT_API_KEY", "MOONSHOT_KEY"],
            url: "https://platform.moonshot.ai/console/api-keys",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [
            ("balance", "Balance"),
            ("cash", "Cash Balance"),
            ("vouchers", "Voucher Balance"),
        ]
        .into_iter()
        .map(|(id, title)| {
            WidgetDescriptor::values(
                format!("{}.{id}", provider.id),
                provider,
                title,
                None,
                None,
                None,
                true,
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
            .ok_or_else(|| http::invalid("The Moonshot API key is missing."))?;
        let remembered = context.memo.get("moonshot.region", context.now).await;
        let first = remembered
            .as_ref()
            .and_then(serde_json::Value::as_str)
            .and_then(|saved| HOSTS.iter().position(|host| *host == saved))
            .unwrap_or(0);
        for index in [first, 1 - first] {
            let host = HOSTS[index];
            let response = http::send(
                context.http,
                HttpRequest::get(format!("{host}/v1/users/me/balance"))
                    .bearer(key)
                    .header("Accept", "application/json"),
                NAME,
            )
            .await?;
            if matches!(response.status, 401 | 403) && index == first {
                continue;
            }
            if !response.is_success() {
                return Err(http::status_error(&response, NAME));
            }
            let body = http::parse(&response, NAME)?;
            if body["code"].as_i64() != Some(0)
                || body["status"].as_bool() != Some(true)
                || body["scode"].as_str().is_none()
            {
                return Err(http::decoding(NAME));
            }
            let mut rows = Vec::new();
            for (title, field) in [
                ("Balance", "available_balance"),
                ("Cash Balance", "cash_balance"),
                ("Voucher Balance", "voucher_balance"),
            ] {
                let amount = body["data"][field]
                    .as_f64()
                    .filter(|amount| amount.is_finite())
                    .ok_or_else(|| http::decoding(NAME))?;
                rows.push(if index == 0 {
                    lines::dollar_value(title, amount)
                } else {
                    lines::count_value(title, amount, "CNY")
                });
            }
            context
                .memo
                .put(
                    "moonshot.region",
                    serde_json::json!(host),
                    Some(context.now + chrono::Duration::hours(12)),
                )
                .await;
            return Ok(Reading::new(None, rows));
        }
        Err(http::expired(
            "Moonshot refused this API key in both regions.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const BODY: &str = r#"{"code":0,"scode":"0","status":true,"data":{"available_balance":12.5,"cash_balance":-2,"voucher_balance":14.5}}"#;

    #[tokio::test]
    async fn a_key_refused_globally_reads_yuan_from_china_and_asks_china_first_afterwards() {
        let http = Scripted::new()
            .on("GET", HOSTS[0], 401, "{}")
            .on("GET", HOSTS[1], 200, BODY);
        let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
        let expected = Reading::new(
            None,
            vec![
                lines::count_value("Balance", 12.5, "CNY"),
                lines::count_value("Cash Balance", -2.0, "CNY"),
                lines::count_value("Voucher Balance", 14.5, "CNY"),
            ],
        );
        assert_eq!(Moonshot.fetch(&scope.context()).await.unwrap(), expected);
        assert_eq!(Moonshot.fetch(&scope.context()).await.unwrap(), expected);
        let requests = http.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2].url, format!("{}/v1/users/me/balance", HOSTS[1]));
        assert_eq!(
            header(&requests[0], "authorization"),
            Some("Bearer fixture")
        );
    }

    #[tokio::test]
    async fn a_global_key_reads_dollars_with_one_request() {
        let http = Scripted::new().on("GET", HOSTS[0], 200, BODY);
        let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
        assert_eq!(
            Moonshot.fetch(&scope.context()).await.unwrap(),
            Reading::new(
                None,
                vec![
                    lines::dollar_value("Balance", 12.5),
                    lines::dollar_value("Cash Balance", -2.0),
                    lines::dollar_value("Voucher Balance", 14.5)
                ]
            )
        );
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn failures_keep_their_categories_and_only_a_refused_key_tries_the_other_region() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{}", ErrorCategory::Decoding),
            (200, "{", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new()
                .on("GET", HOSTS[0], status, body)
                .on("GET", HOSTS[1], status, body);
            let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
            assert_eq!(
                Moonshot.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
            assert_eq!(
                http.requests().len(),
                if matches!(status, 401 | 403) { 2 } else { 1 }
            );
        }
    }
}
