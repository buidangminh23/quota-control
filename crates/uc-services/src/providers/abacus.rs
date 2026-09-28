//! Abacus AI: the compute points an Abacus AI organization has used, shown as monthly credits.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key is the value of the `sessionid`
//! cookie of a signed-in Abacus AI browser session, pasted in Quota Control. A refresh first sends
//! `GET https://apps.abacus.ai/api/_getOrganizationComputePoints` with that cookie, which only
//! reads, and shows the points used (`totalComputePoints` minus `computePointsLeft`) out of
//! `totalComputePoints`. It then sends `POST https://apps.abacus.ai/api/_getBillingInfo` with the
//! same cookie and an empty JSON body: when that succeeds, its `currentTier` names the plan and its
//! `nextBillingDate` becomes the credits' reset time and the plan's renewal date; when it fails, the
//! credits show without them.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, PlanTerm, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Abacus;

const NAME: &str = "Abacus AI";
const URL: &str = "https://apps.abacus.ai/api/_getOrganizationComputePoints";

#[async_trait]
impl Service for Abacus {
    fn id(&self) -> &'static str {
        "abacus"
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
            url: "https://apps.abacus.ai/chatllm/admin/compute-points-usage",
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
            )
            .exporting_progress("credits", "credits"),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Abacus AI session cookie is missing."))?;
        let body = http::json(
            context.http,
            HttpRequest::get(URL)
                .header("Cookie", format!("sessionid={key}"))
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        let mut reading = parse(&body)?;
        if let Ok(billing) = http::json(
            context.http,
            HttpRequest::post("https://apps.abacus.ai/api/_getBillingInfo")
                .header("Cookie", format!("sessionid={key}"))
                .json_body(&serde_json::json!({})),
            NAME,
        )
        .await
            && billing.get("success") == Some(&Value::Bool(true))
        {
            reading.plan = value::text(&billing, "/result/currentTier").and_then(lines::plan_name);
            let next_billing = value::time(&billing, "/result/nextBillingDate");
            if let Some(uc_core::MetricLine::Progress(line)) = reading.lines.first_mut() {
                line.resets_at = next_billing;
            }
            reading.plan_term = next_billing.map(|ends_at| PlanTerm::Stated {
                ends_at,
                checked_at: None,
            });
        }
        Ok(reading)
    }
}

/// The credits row of a compute-points answer: the points used out of the organization's total.
fn parse(body: &Value) -> Result<Reading, SimpleProviderError> {
    if body.get("success") != Some(&Value::Bool(true)) {
        return Err(http::decoding(NAME));
    }
    let total =
        value::number(body, "/result/totalComputePoints").ok_or_else(|| http::decoding(NAME))?;
    let left =
        value::number(body, "/result/computePointsLeft").ok_or_else(|| http::decoding(NAME))?;
    if total < 0.0 {
        return Err(http::decoding(NAME));
    }
    Ok(Reading::new(
        None,
        vec![lines::count(
            "Credits",
            (total - left).max(0.0),
            total,
            "credits",
            None,
            Some(lines::MONTH_MS),
        )],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const COMPUTE_POINTS: &str =
        r###"{"success":true,"result":{"totalComputePoints":20000,"computePointsLeft":15000}}"###;
    const BILLING_INFO: &str = r#"{"success":true,"result":{"currentTier":"pro","nextBillingDate":"2026-10-01T00:00:00Z"}}"#;

    #[tokio::test]
    async fn the_credits_used_come_with_the_billing_tier_as_plan_and_the_next_billing_date() {
        let http = Scripted::new().on("GET", URL, 200, COMPUTE_POINTS);
        let http = http.on(
            "POST",
            "https://apps.abacus.ai/api/_getBillingInfo",
            200,
            BILLING_INFO,
        );
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey":"test"}), now);
        let reading = Abacus.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Pro".into()),
                vec![lines::count(
                    "Credits",
                    5000.0,
                    20000.0,
                    "credits",
                    value::as_time(&json!("2026-10-01T00:00:00Z")),
                    Some(lines::MONTH_MS)
                )]
            )
            .with_plan_term(Some(PlanTerm::Stated {
                ends_at: Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
                checked_at: None,
            }))
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url, URL);
        assert_eq!(header(&requests[0], "Cookie"), Some("sessionid=test"));
        assert_eq!(requests[0].body, None);
    }

    #[tokio::test]
    async fn failed_reads_keep_their_category_and_never_repeat_the_answer() {
        for (status, body, category) in [
            (401, "secret", ErrorCategory::AuthExpired),
            (403, "secret", ErrorCategory::AuthExpired),
            (429, "", ErrorCategory::RateLimited),
            (500, "", ErrorCategory::Http5xx),
            (200, "{}", ErrorCategory::Decoding),
            (200, "bad json", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            let error = Abacus.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            Abacus.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
