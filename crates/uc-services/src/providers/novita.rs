//! Novita AI: the available balance of a Novita AI API key in dollars, with the account's cash,
//! credit limit, pending charges and outstanding invoices when Novita lists them.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the `NOVITA_API_KEY`
//! environment variable or from a key saved in Quota Control. A refresh sends one read-only
//! request, `GET https://api.novita.ai/openapi/v1/billing/balance/detail` with the key as a bearer
//! token. Novita documents its amounts in ten-thousandths of a dollar
//! (https://novita.ai/docs/api-reference/basic-get-user-balance.md), so each one is divided by
//! 10,000 before it is shown.

use async_trait::async_trait;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Novita;

const URL: &str = "https://api.novita.ai/openapi/v1/billing/balance/detail";

#[async_trait]
impl Service for Novita {
    fn id(&self) -> &'static str {
        "novita"
    }

    fn name(&self) -> &'static str {
        "Novita AI"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["NOVITA_API_KEY"],
            url: "https://novita.ai/settings/key-management",
            fields: &[],
        })
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
            .ok_or_else(|| http::invalid("The Novita AI API key is missing."))?;
        let body = http::json(context.http, HttpRequest::get(URL).bearer(key), "Novita AI").await?;
        let balance =
            value::number(&body, "/availableBalance").ok_or_else(|| http::decoding("Novita AI"))?;
        let mut rows = vec![lines::dollar_value("Balance", balance / 10000.0)];
        for (field, label) in [
            ("/cashBalance", "Cash"),
            ("/creditLimit", "Credit limit"),
            ("/pendingCharges", "Pending charges"),
            ("/outstandingInvoices", "Outstanding invoices"),
        ] {
            if let Some(amount) = value::number(&body, field) {
                rows.push(lines::dollar_value(label, amount / 10000.0));
            }
        }
        Ok(Reading::new(None, rows))
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
    async fn every_documented_amount_shows_in_dollars_with_the_key_as_a_bearer() {
        let body = r#"{"availableBalance":"1000000","cashBalance":"800000","creditLimit":"200000","pendingCharges":"0","outstandingInvoices":"0"}"#;
        let http = Scripted::new().on("GET", URL, 200, body);
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        let reading = Novita.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![
                lines::dollar_value("Balance", 100.0),
                lines::dollar_value("Cash", 80.0),
                lines::dollar_value("Credit limit", 20.0),
                lines::dollar_value("Pending charges", 0.0),
                lines::dollar_value("Outstanding invoices", 0.0)
            ]
        );
        assert_eq!(http.requests().len(), 1);
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some("Bearer test")
        );
    }

    #[tokio::test]
    async fn refused_keys_rate_limits_server_errors_and_missing_balances_keep_their_categories() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (503, ErrorCategory::Http5xx),
            (200, ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", URL, status, "{}");
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            assert_eq!(
                Novita.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }

    #[tokio::test]
    async fn a_secret_without_a_key_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            Novita.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
