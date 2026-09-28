//! Augment: the credits of an Augment account, read through its web session.
//!
//! Nothing is read from disk or from the environment on Windows, macOS or Linux: the card uses the
//! `session` cookie of app.augmentcode.com pasted into Accounts. A refresh sends
//! `GET https://app.augmentcode.com/api/credits` with that cookie for the Credits meter (the
//! credits used this billing cycle against those available) and the Balance row (the credits
//! left), then `GET https://app.augmentcode.com/api/subscription` with the same cookie for the
//! plan name, the account's `email` and the end of the billing period, which becomes the meter's
//! reset time and the plan's renewal date. When that second request fails, the credits are still
//! shown, without a plan or a reset time.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, PlanTerm, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Augment;

const NAME: &str = "Augment";
const CREDITS_URL: &str = "https://app.augmentcode.com/api/credits";

#[async_trait]
impl Service for Augment {
    fn id(&self) -> &'static str {
        "augment"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn key_label(&self) -> &'static str {
        "Session cookie (session)"
    }

    fn key_format(&self) -> KeyFormat {
        KeyFormat::Cookie
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "https://app.augmentcode.com/account/subscription",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::bounded_count(
                format!("{}.credits", provider.id),
                provider,
                "Credits",
                None,
                0.0,
                "credits",
                Some(lines::MONTH_MS),
            ),
            WidgetDescriptor::values(
                format!("{}.balance", provider.id),
                provider,
                "Balance",
                None,
                Some(uc_core::MetricKind::Count),
                Some("left"),
                false,
                None,
                false,
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Augment session cookie is missing."))?;
        let body = http::json(
            context.http,
            HttpRequest::get(CREDITS_URL)
                .header("Cookie", format!("session={key}"))
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        let mut reading = parse(&body)?;
        if let Ok(subscription) = http::json(
            context.http,
            HttpRequest::get("https://app.augmentcode.com/api/subscription")
                .header("Cookie", format!("session={key}"))
                .header("Accept", "application/json"),
            NAME,
        )
        .await
        {
            reading.plan = value::text(&subscription, "/planName").and_then(lines::plan_name);
            let period_end = value::time(&subscription, "/billingPeriodEnd");
            if let Some(uc_core::MetricLine::Progress(line)) = reading.lines.first_mut() {
                line.resets_at = period_end;
            }
            reading = reading
                .with_plan_term(period_end.map(|ends_at| PlanTerm::Stated {
                    ends_at,
                    checked_at: None,
                }))
                .with_account(value::text(&subscription, "/email"));
        }
        Ok(reading)
    }
}

/// The Credits meter and the Balance row of an `/api/credits` answer. A total that is missing or
/// zero is worked out as used plus remaining, and a missing used figure as total minus remaining;
/// an answer that gives neither row is a decoding error.
fn parse(body: &Value) -> Result<Reading, SimpleProviderError> {
    let left = value::number(body, "/usageUnitsRemaining");
    let used = value::number(body, "/usageUnitsConsumedThisBillingCycle");
    let limit = value::number(body, "/usageUnitsAvailable")
        .filter(|available| *available > 0.0)
        .or_else(|| used.zip(left).map(|(used, left)| used + left));
    let mut rows = Vec::new();
    if let (Some(used), Some(limit)) = (
        used.or_else(|| limit.zip(left).map(|(limit, left)| limit - left)),
        limit,
    ) {
        rows.push(lines::count(
            "Credits",
            used,
            limit,
            "credits",
            None,
            Some(lines::MONTH_MS),
        ));
    }
    if let Some(left) = left {
        rows.push(lines::count_value("Balance", left, "credits"));
    }
    if rows.is_empty() {
        return Err(http::decoding(NAME));
    }
    Ok(Reading::new(None, rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const CREDITS: &str = r###"{"usageUnitsRemaining":800,"usageUnitsConsumedThisBillingCycle":200,"usageUnitsAvailable":1000}"###;

    #[tokio::test]
    async fn reads_credits_balance_and_plan_with_the_session_cookie() {
        let http = Scripted::new().on("GET", CREDITS_URL, 200, CREDITS).on(
            "GET",
            "https://app.augmentcode.com/api/subscription",
            200,
            r#"{"planName":"pro","billingPeriodEnd":"2026-10-01T00:00:00Z","email":"fixture@example.com"}"#,
        );
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey":"test"}), now);
        let reading = Augment.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::count(
                        "Credits",
                        200.0,
                        1000.0,
                        "credits",
                        value::as_time(&json!("2026-10-01T00:00:00Z")),
                        Some(lines::MONTH_MS)
                    ),
                    lines::count_value("Balance", 800.0, "credits")
                ]
            )
            .with_plan_term(Some(PlanTerm::Stated {
                ends_at: Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
                checked_at: None,
            }))
            .with_account(Some("fixture@example.com"))
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url, CREDITS_URL);
        assert_eq!(header(&requests[0], "Cookie"), Some("session=test"));
        assert_eq!(requests[0].body, None);
    }

    #[tokio::test]
    async fn each_failure_keeps_its_category_without_repeating_the_answer() {
        for (status, body, category) in [
            (401, "secret", ErrorCategory::AuthExpired),
            (403, "secret", ErrorCategory::AuthExpired),
            (429, "", ErrorCategory::RateLimited),
            (500, "", ErrorCategory::Http5xx),
            (200, "{}", ErrorCategory::Decoding),
            (200, "bad json", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", CREDITS_URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            let error = Augment.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            Augment.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
