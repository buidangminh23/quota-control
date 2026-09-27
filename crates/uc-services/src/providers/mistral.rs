//! Mistral: the credit balance of a Mistral organization and this month's spend, in total and per
//! model or resource, in the currency Mistral bills in.
//!
//! Nothing is read from disk or from a browser's cookie store on Windows, macOS or Linux: the user
//! pastes the Cookie header of a signed-in `https://admin.mistral.ai` session, which must hold an
//! `ory_session_*` cookie, and may add the value of its `csrftoken` cookie. A refresh sends two
//! requests to that site, `GET /api/billing/credits` and then
//! `GET /api/billing/v2/usage?month=<month>&year=<year>` for the current UTC month, each carrying
//! the pasted header as `Cookie`, the site as `Origin`, its organization usage page as `Referer`
//! and, when one is saved, the CSRF value as `X-CSRFTOKEN`.
//!
//! The balance is `wallet_amount` plus `credit_notes_amount` minus `ongoing_usage_balance`. The
//! month's spend prices every input, cached and output entry of every model the usage answer lists
//! (its paid units, else its units) at the price listed for the entry's billing metric and group,
//! and each resource (the entry's display name, else its model's name) gets a row with its own
//! spend. Amounts in US dollars show as dollars, others with their currency's code.

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::Datelike;
use serde_json::Value;
use uc_core::{HttpRequest, MetricLine, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Mistral;

const BASE: &str = "https://admin.mistral.ai";

#[async_trait]
impl Service for Mistral {
    fn id(&self) -> &'static str {
        "mistral"
    }

    fn name(&self) -> &'static str {
        "Mistral"
    }

    fn key_label(&self) -> &'static str {
        "Cookie header (ory_session_*)"
    }

    fn key_format(&self) -> KeyFormat {
        KeyFormat::CookieHeader
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: BASE,
            fields: &[("csrfToken", "CSRF cookie (csrftoken)")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [("balance", "Balance"), ("monthly", "Spend This Month")]
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
        let cookie = context
            .secret
            .key()
            .filter(|header| {
                header.contains("ory_session_")
                    && header.contains('=')
                    && !header.chars().any(char::is_control)
            })
            .ok_or_else(|| http::invalid("Paste the Mistral ory_session cookie header."))?;
        let credits = get(context, cookie, "/api/billing/credits").await?;
        let wallet =
            value::number(&credits, "/wallet_amount").ok_or_else(|| http::decoding("Mistral"))?;
        let balance = wallet + value::number(&credits, "/credit_notes_amount").unwrap_or(0.0)
            - value::number(&credits, "/ongoing_usage_balance").unwrap_or(0.0);
        let currency =
            value::text(&credits, "/currency").ok_or_else(|| http::decoding("Mistral"))?;
        if !balance.is_finite() {
            return Err(http::decoding("Mistral"));
        }
        let usage = get(
            context,
            cookie,
            &format!(
                "/api/billing/v2/usage?month={}&year={}",
                context.now.month(),
                context.now.year()
            ),
        )
        .await?;
        let usage_currency =
            value::text(&usage, "/currency").ok_or_else(|| http::decoding("Mistral"))?;
        let prices = usage
            .get("prices")
            .and_then(Value::as_array)
            .ok_or_else(|| http::decoding("Mistral"))?;
        let mut total = 0.0;
        let mut resources = BTreeMap::<String, f64>::new();
        let mut seen = false;
        for path in [
            "/completion/models",
            "/chat/models",
            "/vibe_code/completion/models",
            "/ocr/models",
            "/connectors/models",
            "/audio/models",
            "/libraries_api/pages/models",
            "/libraries_api/tokens/models",
            "/fine_tuning/training",
            "/fine_tuning/storage",
        ] {
            if let Some(models) = usage.pointer(path).and_then(Value::as_object) {
                seen = true;
                for (name, data) in models {
                    for kind in ["input", "cached", "output"] {
                        if let Some(entries) = data.get(kind).and_then(Value::as_array) {
                            for entry in entries {
                                let units = value::number(entry, "/value_paid")
                                    .or_else(|| value::number(entry, "/value"))
                                    .ok_or_else(|| http::decoding("Mistral"))?;
                                let metric = value::text(entry, "/billing_metric")
                                    .ok_or_else(|| http::decoding("Mistral"))?;
                                let group = value::text(entry, "/billing_group")
                                    .ok_or_else(|| http::decoding("Mistral"))?;
                                let price = prices
                                    .iter()
                                    .find(|price| {
                                        value::text(price, "/billing_metric") == Some(metric)
                                            && value::text(price, "/billing_group") == Some(group)
                                    })
                                    .and_then(|price| value::number(price, "/price"))
                                    .ok_or_else(|| http::decoding("Mistral"))?;
                                let cost = units * price;
                                if !cost.is_finite() {
                                    return Err(http::decoding("Mistral"));
                                }
                                total += cost;
                                let display =
                                    value::text(entry, "/billing_display_name").unwrap_or(name);
                                *resources.entry(display.into()).or_default() += cost;
                            }
                        }
                    }
                }
            }
        }
        if !seen || !total.is_finite() {
            return Err(http::decoding("Mistral"));
        }
        let mut rows = vec![
            money("Balance", balance, currency),
            money("Spend This Month", total, usage_currency),
        ];
        for (name, cost) in resources {
            rows.push(money(&name, cost, usage_currency));
        }
        Ok(Reading::new(None, rows))
    }
}

/// An amount as dollars when `currency` is US dollars, else as a count in that currency's code.
fn money(label: &str, amount: f64, currency: &str) -> MetricLine {
    if currency.eq_ignore_ascii_case("USD") {
        lines::dollar_value(label, amount)
    } else {
        lines::count_value(label, amount, currency)
    }
}

/// A GET of `path` on the admin site, sent with the pasted cookies, the site's Origin and Referer,
/// and the saved CSRF value when there is one.
async fn get(
    context: &FetchContext<'_>,
    cookie: &str,
    path: &str,
) -> Result<Value, SimpleProviderError> {
    let mut request = HttpRequest::get(format!("{BASE}{path}"))
        .header("Cookie", cookie)
        .header("Origin", BASE)
        .header("Referer", "https://admin.mistral.ai/organization/usage");
    if let Some(csrf) = context.secret.str("/csrfToken") {
        if csrf.chars().any(char::is_control) {
            return Err(http::invalid("The Mistral CSRF cookie is invalid."));
        }
        request = request.header("X-CSRFTOKEN", csrf);
    }
    http::json(context.http, request, "Mistral").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const CREDITS: &str = r#"{"wallet_amount":20,"credit_notes_amount":5,"ongoing_usage_balance":3,"currency":"EUR"}"#;
    const USAGE: &str = r#"{"currency":"EUR","completion":{"models":{"mistral-small":{"input":[{"billing_metric":"tokens","billing_group":"input","billing_display_name":"Mistral Small","value":1000,"value_paid":800}]}}},"prices":[{"billing_metric":"tokens","billing_group":"input","price":"0.001"}]}"#;

    #[tokio::test]
    async fn reads_the_balance_and_this_months_spend_per_resource_with_the_cookie_and_csrf_token() {
        let http = Scripted::new()
            .on("GET", &format!("{BASE}/api/billing/credits"), 200, CREDITS)
            .on("GET", &format!("{BASE}/api/billing/v2/usage"), 200, USAGE);
        let scope = context_at(
            &http,
            json!({"apiKey":"ory_session_test=t","csrfToken":"csrf"}),
            Utc.with_ymd_and_hms(2026, 9, 27, 0, 0, 0).unwrap(),
        );
        let reading = Mistral.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![
                money("Balance", 22.0, "EUR"),
                money("Spend This Month", 0.8, "EUR"),
                money("Mistral Small", 0.8, "EUR")
            ]
        );
        assert_eq!(reading.plan, None);
        assert_eq!(
            header(&http.requests()[0], "Cookie"),
            Some("ory_session_test=t")
        );
        assert_eq!(header(&http.requests()[0], "X-CSRFTOKEN"), Some("csrf"));
        assert!(http.requests()[1].url.ends_with("month=9&year=2026"));
    }

    #[tokio::test]
    async fn refused_rate_limited_and_unreadable_answers_keep_their_categories() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (200, ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", BASE, status, "{}");
            let scope = context_at(&http, json!({"apiKey":"ory_session_test=t"}), Utc::now());
            assert_eq!(
                Mistral.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }
}
