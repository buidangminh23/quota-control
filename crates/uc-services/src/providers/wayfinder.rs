//! Wayfinder: the requests, tokens and savings of a Wayfinder router over the last 30 days.
//!
//! Nothing is read from disk on Windows, macOS or Linux and no credential is sent: the card uses
//! the server address saved in Accounts, which must use https://, or plain http:// to this
//! computer, and the key field only labels the connection. A refresh sends one request,
//! `GET <server>/v1/savings?period=30d`, and shows the request and token totals, the savings (in
//! dollars when the answer is priced, otherwise as a percentage) and one row per route with that
//! route's requests and tokens.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, MetricKind, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{endpoint, http, lines, value};

pub(crate) struct Wayfinder;

#[async_trait]
impl Service for Wayfinder {
    fn id(&self) -> &'static str {
        "wayfinder"
    }

    fn name(&self) -> &'static str {
        "Wayfinder"
    }

    fn key_label(&self) -> &'static str {
        "Connection label (not sent)"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "http://127.0.0.1:8088/router",
            fields: &[("baseUrl", "Server address")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [("requests", "Requests"), ("tokens", "Tokens")]
            .into_iter()
            .map(|(id, title)| {
                WidgetDescriptor::values(
                    format!("{}.{id}", provider.id),
                    provider,
                    title,
                    None,
                    Some(MetricKind::Count),
                    None,
                    false,
                    None,
                    false,
                )
            })
            .chain(std::iter::once(WidgetDescriptor::combined(
                format!("{}.savings", provider.id),
                provider,
                "Savings",
                None,
                false,
            )))
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let base = endpoint::base_url(
            context.secret.str("/baseUrl"),
            None,
            endpoint::Policy::HttpsOrLoopbackHttp,
            "Wayfinder",
        )?;
        let body = http::json(
            context.http,
            HttpRequest::get(format!("{base}/v1/savings?period=30d")),
            "Wayfinder",
        )
        .await?;
        let requests =
            value::number(&body, "/requests").ok_or_else(|| http::decoding("Wayfinder"))?;
        let tokens = value::number(&body, "/tokens").ok_or_else(|| http::decoding("Wayfinder"))?;
        let mut rows = vec![
            lines::count_value("Requests", requests, "requests"),
            lines::count_value("Tokens", tokens, "tokens"),
        ];
        if value::flag(&body, "/priced") == Some(true) {
            if let Some(saved) = value::number(&body, "/saved") {
                rows.push(lines::dollar_value("Savings", saved));
            }
        } else if let Some(percent) = value::number(&body, "/saved_pct") {
            rows.push(lines::count_value("Savings", percent, "%"));
        }
        if let Some(routes) = body.get("by_route").and_then(Value::as_object) {
            for (name, route) in routes {
                let mut values = vec![];
                if let Some(count) = value::number(route, "/requests") {
                    values.push(uc_core::MetricValue::count(count, "requests"));
                }
                if let Some(count) = value::number(route, "/tokens") {
                    values.push(uc_core::MetricValue::count(count, "tokens"));
                }
                if !values.is_empty() {
                    rows.push(lines::values(name, values));
                }
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

    const SAVINGS: &str = r#"{"requests":10,"tokens":1000,"priced":true,"saved":2.5,"by_route":{"local":{"requests":8,"tokens":800}}}"#;

    #[tokio::test]
    async fn reads_totals_dollar_savings_and_routes_without_sending_the_label() {
        let http = Scripted::new().on(
            "GET",
            "http://127.0.0.1:8088/v1/savings?period=30d",
            200,
            SAVINGS,
        );
        let scope = context_at(
            &http,
            json!({"baseUrl":"http://127.0.0.1:8088","apiKey":"do-not-send"}),
            Utc::now(),
        );
        let reading = Wayfinder.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![
                lines::count_value("Requests", 10.0, "requests"),
                lines::count_value("Tokens", 1000.0, "tokens"),
                lines::dollar_value("Savings", 2.5),
                lines::values(
                    "local",
                    vec![
                        uc_core::MetricValue::count(8.0, "requests"),
                        uc_core::MetricValue::count(800.0, "tokens")
                    ]
                )
            ]
        );
        assert_eq!(header(&http.requests()[0], "Authorization"), None);
        assert_eq!(reading.plan, None);
    }

    #[tokio::test]
    async fn refused_rate_limited_and_unreadable_answers_keep_their_categories() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (200, ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", "https://gateway.test", status, "{}");
            let scope = context_at(&http, json!({"baseUrl":"https://gateway.test"}), Utc::now());
            assert_eq!(
                Wayfinder
                    .fetch(&scope.context())
                    .await
                    .unwrap_err()
                    .category,
                category
            );
        }
    }

    #[tokio::test]
    async fn plain_http_to_another_computer_is_refused_before_any_request() {
        let http = Scripted::new();
        let scope = context_at(
            &http,
            json!({"baseUrl":"http://public.example"}),
            Utc::now(),
        );
        assert_eq!(
            Wayfinder
                .fetch(&scope.context())
                .await
                .unwrap_err()
                .category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
