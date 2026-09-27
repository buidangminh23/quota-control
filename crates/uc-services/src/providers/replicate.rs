//! Replicate: this month's spend and the unused credit of a Replicate user or organization.
//!
//! Nothing is read from disk or the environment on Windows, macOS or Linux: the key is the value
//! of the `sessionid` session cookie, saved in Quota Control with the account's user name and
//! whether it is an organization. A refresh sends the cookie with
//! `GET https://replicate.com/api/users/{account}/invoices` (`/api/organizations/{account}` for an
//! organization) and reads the cost before adjustments of the monthly-usage invoice that has not
//! ended, then asks the same account's `/unused-credit` and shows a Balance row when that answer
//! can be read.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Replicate;

const NAME: &str = "Replicate";
#[cfg(test)]
const URL: &str = "https://replicate.com/api/users/example/invoices";

#[async_trait]
impl Service for Replicate {
    fn id(&self) -> &'static str {
        "replicate"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn key_label(&self) -> &'static str {
        "Session cookie (sessionid)"
    }

    fn key_format(&self) -> KeyFormat {
        KeyFormat::Cookie
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "https://replicate.com/account/billing",
            fields: &[
                ("account", "Account username"),
                ("organization", "Organization account (true or false)"),
            ],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::combined(
                format!("{}.month", provider.id),
                provider,
                "Spend This Month",
                None,
                true,
            ),
            WidgetDescriptor::dollar_balance(
                format!("{}.balance", provider.id),
                provider,
                "Balance",
                None,
                "left",
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Replicate session cookie is missing."))?;
        let account = context
            .secret
            .str("/account")
            .ok_or_else(|| http::invalid("Enter the Replicate account username."))?;
        let segment = url::form_urlencoded::byte_serialize(account.as_bytes()).collect::<String>();
        let kind = if context.secret.str("/organization") == Some("true") {
            "organizations"
        } else {
            "users"
        };
        let base = format!("https://replicate.com/api/{kind}/{segment}");
        let body = http::json(
            context.http,
            HttpRequest::get(format!("{base}/invoices"))
                .header("Cookie", format!("sessionid={key}")),
            NAME,
        )
        .await?;
        let mut reading = parse(&body, context)?;
        if let Ok(credit) = http::json(
            context.http,
            HttpRequest::get(format!("{base}/unused-credit"))
                .header("Cookie", format!("sessionid={key}")),
            NAME,
        )
        .await
            && let Some(balance) = value::number(&credit, "/unused_credit")
        {
            reading.lines.push(lines::dollar_value("Balance", balance));
        }
        Ok(reading)
    }
}

/// This month's spend: the cost before adjustments of the monthly-usage invoice that has not
/// ended yet.
fn parse(body: &Value, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
    let invoices = body
        .get("invoices")
        .and_then(Value::as_array)
        .ok_or_else(|| http::decoding(NAME))?;
    let current = invoices
        .iter()
        .find(|invoice| {
            value::text(invoice, "/type") == Some("monthly-usage")
                && (invoice.get("ended_before").is_none_or(Value::is_null)
                    || value::time(invoice, "/ended_before")
                        .is_some_and(|ended| ended > context.now))
        })
        .ok_or_else(|| http::decoding(NAME))?;
    let used = value::number(current, "/total_cost_before_adjustments")
        .filter(|cost| *cost >= 0.0)
        .ok_or_else(|| http::decoding(NAME))?;
    Ok(Reading::new(
        None,
        vec![lines::dollar_value("Spend This Month", used)],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const INVOICES: &str = r###"{"invoices":[{"type":"monthly-usage","ended_before":null,"total_cost_before_adjustments":"12.34"}]}"###;

    #[tokio::test]
    async fn shows_the_open_invoice_spend_and_the_unused_credit_with_the_session_cookie() {
        let http = Scripted::new().on("GET", URL, 200, INVOICES);
        let http = http.on(
            "GET",
            "https://replicate.com/api/users/example/unused-credit",
            200,
            r#"{"unused_credit":"5.00"}"#,
        );
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey": "test", "account": "example"}), now);
        let reading = Replicate.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                None,
                vec![
                    lines::dollar_value("Spend This Month", 12.34),
                    lines::dollar_value("Balance", 5.0)
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url, URL);
        assert_eq!(header(&requests[0], "Cookie"), Some("sessionid=test"));
        assert_eq!(requests[0].body, None);
    }

    #[tokio::test]
    async fn failed_or_unreadable_answers_keep_their_category_and_hide_the_body() {
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
                json!({"apiKey": "test", "account": "example"}),
                Utc::now(),
            );
            let error = Replicate.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            Replicate
                .fetch(&scope.context())
                .await
                .unwrap_err()
                .category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
