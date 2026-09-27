//! Perplexity: the monthly, bonus and purchased credits of a Perplexity account, in dollars.
//!
//! Nothing is read from disk or the environment on Windows, macOS or Linux: the key is the value
//! of the `__Secure-authjs.session-token` session cookie, saved in Quota Control. A refresh sends
//! that cookie, with the site's `Origin` and `Referer`, in one request:
//! `GET https://www.perplexity.ai/rest/billing/credits?version=2.18&source=default`. Usage is
//! charged to the monthly grants first, then to purchased credits, then to the promotional grants
//! that have not expired, which make up the Bonus meter. The plan reads Pro when the monthly
//! grants come to less than $50, Max otherwise.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Perplexity;

const NAME: &str = "Perplexity";
const URL: &str = "https://www.perplexity.ai/rest/billing/credits?version=2.18&source=default";

#[async_trait]
impl Service for Perplexity {
    fn id(&self) -> &'static str {
        "perplexity"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn key_label(&self) -> &'static str {
        "Session cookie (__Secure-authjs.session-token)"
    }

    fn key_format(&self) -> KeyFormat {
        KeyFormat::Cookie
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "https://www.perplexity.ai/account/usage",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        ["Monthly", "Bonus", "Purchased"]
            .into_iter()
            .zip(["monthly", "bonus", "purchased"])
            .map(|(title, id)| {
                WidgetDescriptor::bounded_dollars(
                    format!("{}.{id}", provider.id),
                    provider,
                    title,
                    None,
                    0.0,
                    None,
                    None,
                )
                .exporting_progress(id, "usd")
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Perplexity session cookie is missing."))?;
        let body = http::json(
            context.http,
            HttpRequest::get(URL)
                .header("Cookie", format!("__Secure-authjs.session-token={key}"))
                .header("Origin", "https://www.perplexity.ai")
                .header("Referer", "https://www.perplexity.ai/account/usage"),
            NAME,
        )
        .await?;
        parse(&body, context)
    }
}

/// The credit meters of an answer, whose fields come in snake_case or in camelCase.
fn parse(body: &Value, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
    let number = |snake: &str, camel: &str| {
        value::number(body, snake)
            .or_else(|| value::number(body, camel))
            .ok_or_else(|| http::decoding(NAME))
    };
    let _balance = number("/balance_cents", "/balanceCents")?;
    let purchased_field = number(
        "/current_period_purchased_cents",
        "/currentPeriodPurchasedCents",
    )?;
    let mut used = number("/total_usage_cents", "/totalUsageCents")?.max(0.0);
    let reset =
        value::time(body, "/renewal_date_ts").or_else(|| value::time(body, "/renewalDateTs"));
    let grants = body
        .get("credit_grants")
        .or_else(|| body.get("creditGrants"))
        .and_then(Value::as_array)
        .ok_or_else(|| http::decoding(NAME))?;
    let (mut recurring, mut bonus, mut purchased) = (0.0, 0.0, 0.0);
    let mut bonus_reset = None;
    for grant in grants {
        let amount = value::number(grant, "/amount_cents")
            .or_else(|| value::number(grant, "/amountCents"))
            .ok_or_else(|| http::decoding(NAME))?;
        let expiry =
            value::time(grant, "/expires_at_ts").or_else(|| value::time(grant, "/expiresAtTs"));
        match value::text(grant, "/type").ok_or_else(|| http::decoding(NAME))? {
            "recurring" => recurring += amount,
            "purchased" => purchased += amount,
            "promotional" if expiry.is_none_or(|expires| expires > context.now) => {
                bonus += amount;
                bonus_reset = match (bonus_reset, expiry) {
                    (Some(soonest), Some(expires)) => Some(std::cmp::min(soonest, expires)),
                    (soonest, expires) => soonest.or(expires),
                };
            }
            _ => {}
        }
    }
    recurring = recurring.max(0.0);
    bonus = bonus.max(0.0);
    purchased = purchased.max(purchased_field).max(0.0);
    let recurring_used = used.min(recurring);
    used -= recurring_used;
    let purchased_used = used.min(purchased);
    used -= purchased_used;
    let bonus_used = used.min(bonus);
    let mut rows = Vec::new();
    if recurring > 0.0 || (bonus == 0.0 && purchased == 0.0) {
        rows.push(lines::dollars(
            "Monthly",
            recurring_used / 100.0,
            recurring / 100.0,
            reset,
            Some(lines::MONTH_MS),
        ));
    }
    if bonus > 0.0 {
        rows.push(lines::dollars(
            "Bonus",
            bonus_used / 100.0,
            bonus / 100.0,
            bonus_reset,
            None,
        ));
    }
    if purchased > 0.0 {
        rows.push(lines::dollars(
            "Purchased",
            purchased_used / 100.0,
            purchased / 100.0,
            None,
            None,
        ));
    }
    Ok(Reading::new(
        (recurring > 0.0).then(|| if recurring < 5000.0 { "Pro" } else { "Max" }.into()),
        rows,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const CREDITS: &str = r###"{"balance_cents":4200,"renewal_date_ts":1790812800,"current_period_purchased_cents":1000,"total_usage_cents":3500,"credit_grants":[{"type":"recurring","amount_cents":3000},{"type":"purchased","amount_cents":1000},{"type":"promotional","amount_cents":500,"expires_at_ts":1790812800}]}"###;

    #[tokio::test]
    async fn charges_usage_to_monthly_then_purchased_then_bonus_credits() {
        let http = Scripted::new().on("GET", URL, 200, CREDITS);
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey": "test"}), now);
        let reading = Perplexity.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::dollars(
                        "Monthly",
                        30.0,
                        30.0,
                        value::as_time(&json!(1790812800)),
                        Some(lines::MONTH_MS)
                    ),
                    lines::dollars("Bonus", 0.0, 5.0, value::as_time(&json!(1790812800)), None),
                    lines::dollars("Purchased", 5.0, 10.0, None, None)
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, URL);
        assert_eq!(
            header(&requests[0], "Cookie"),
            Some("__Secure-authjs.session-token=test")
        );
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
            let scope = context_at(&http, json!({"apiKey": "test"}), Utc::now());
            let error = Perplexity.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            Perplexity
                .fetch(&scope.context())
                .await
                .unwrap_err()
                .category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
