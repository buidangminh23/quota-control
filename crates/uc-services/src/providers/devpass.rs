//! DevPass: the credits and weekly premium quota of a DevPass plan with the balance left, and the
//! spend and limit of the LLM Gateway key that reads them.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the `DEVPASS_API_KEY`
//! environment variable or from a DevPass API key saved in Quota Control. A refresh sends one
//! request, `GET https://api.llmgateway.io/v1/key` with the key as a bearer token, and reads the
//! `devPlan` tier with its `devPlanCredits*` and `devPlanPremium*` amounts, and the key's `usage`
//! and `limit`. Amounts arrive as decimal strings. A key without a plan (`none`) shows only its
//! spend and, when it has one, its limit, under the Pay As You Go plan.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, MetricKind, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct DevPass;

const NAME: &str = "DevPass";
const URL: &str = "https://api.llmgateway.io/v1/key";

#[async_trait]
impl Service for DevPass {
    fn id(&self) -> &'static str {
        "devpass"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn key_label(&self) -> &'static str {
        "DevPass API key"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["DEVPASS_API_KEY"],
            url: "https://llmgateway.io/dashboard",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let money = |id: &str, title: &str| {
            WidgetDescriptor::values(
                format!("{}.{id}", provider.id),
                provider,
                title,
                None,
                Some(MetricKind::Dollars),
                None,
                true,
                None,
                false,
            )
        };
        vec![
            WidgetDescriptor::bounded_dollars(
                format!("{}.credits", provider.id),
                provider,
                "Credits",
                None,
                100.0,
                None,
                None,
            )
            .exporting_progress("credits", "usd"),
            WidgetDescriptor::bounded_dollars(
                format!("{}.weekly", provider.id),
                provider,
                "Weekly",
                None,
                100.0,
                None,
                None,
            )
            .exporting_progress("weekly", "usd"),
            money("balance", "Balance"),
            money("total", "Total Usage"),
            money("keyLimit", "Key Limit"),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The DevPass API key is missing."))?;
        let body = http::json(
            context.http,
            HttpRequest::get(URL)
                .bearer(key)
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        let data = &body["data"];
        let tier = data["devPlan"]
            .as_str()
            .filter(|tier| matches!(*tier, "none" | "lite" | "pro" | "max"))
            .ok_or_else(|| http::decoding(NAME))?;
        let key_used = money(&data["usage"])?;
        let key_limit = if data["limit"].is_null() {
            None
        } else {
            Some(money(&data["limit"])?)
        };
        let mut rows = Vec::new();
        if tier != "none" {
            let used = money(&data["devPlanCreditsUsed"])?;
            let limit = money(&data["devPlanCreditsLimit"])?;
            let remaining = money(&data["devPlanCreditsRemaining"])?;
            let weekly = money(&data["devPlanPremiumCreditsUsed"])?;
            let weekly_limit = money(&data["devPlanPremiumWeeklyLimit"])?;
            let reset = if data["devPlanPremiumWeekResetsAt"].is_null() {
                None
            } else {
                Some(
                    value::time(data, "/devPlanPremiumWeekResetsAt")
                        .ok_or_else(|| http::decoding(NAME))?,
                )
            };
            if limit > 0.0 {
                rows.push(lines::dollars("Credits", used, limit, None, None));
            }
            if weekly_limit > 0.0 {
                rows.push(lines::dollars(
                    "Weekly",
                    weekly,
                    weekly_limit,
                    reset,
                    Some(lines::WEEK_MS),
                ));
            }
            rows.push(lines::dollar_value("Balance", remaining));
        }
        rows.push(lines::dollar_value("Total Usage", key_used));
        if let Some(limit) = key_limit {
            rows.push(lines::dollar_value("Key Limit", limit));
        }
        let plan = if tier == "none" {
            "Pay As You Go".into()
        } else {
            format!("DevPass {}", lines::plan_name(tier).unwrap_or_default())
        };
        Ok(Reading::new(Some(plan), rows))
    }
}

/// A dollar amount written as a decimal string of digits with at most one `.`; anything else,
/// including a JSON number, cannot be read.
fn money(amount: &Value) -> Result<f64, SimpleProviderError> {
    let raw = amount.as_str().ok_or_else(|| http::decoding(NAME))?;
    let mut parts = raw.split('.');
    let integer = parts.next().unwrap_or_default();
    if integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || parts.next().is_some_and(|fraction| {
            fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
        || parts.next().is_some()
    {
        return Err(http::decoding(NAME));
    }
    raw.parse::<f64>()
        .ok()
        .filter(|parsed| parsed.is_finite())
        .ok_or_else(|| http::decoding(NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use serde_json::json;
    use uc_core::ErrorCategory;

    #[tokio::test]
    async fn a_plan_key_shows_credits_the_weekly_quota_its_balance_and_the_key_spend() {
        let body = r#"{"data":{"devPlan":"pro","usage":"9.5","limit":"100","devPlanCreditsUsed":"2.5","devPlanCreditsLimit":"20","devPlanCreditsRemaining":"17.5","devPlanPremiumCreditsUsed":"1","devPlanPremiumWeeklyLimit":"5","devPlanPremiumWeekResetsAt":"2026-10-01T00:00:00Z"}}"#;
        let http = Scripted::new().on("GET", URL, 200, body);
        let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
        assert_eq!(
            DevPass.fetch(&scope.context()).await.unwrap(),
            Reading::new(
                Some("DevPass Pro".into()),
                vec![
                    lines::dollars("Credits", 2.5, 20.0, None, None),
                    lines::dollars(
                        "Weekly",
                        1.0,
                        5.0,
                        Some("2026-10-01T00:00:00Z".parse().unwrap()),
                        Some(lines::WEEK_MS)
                    ),
                    lines::dollar_value("Balance", 17.5),
                    lines::dollar_value("Total Usage", 9.5),
                    lines::dollar_value("Key Limit", 100.0)
                ]
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

    #[tokio::test]
    async fn a_pay_as_you_go_key_shows_its_spend_without_an_invented_allowance() {
        let http = Scripted::new().on(
            "GET",
            URL,
            200,
            r#"{"data":{"devPlan":"none","usage":"3","limit":null}}"#,
        );
        let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
        assert_eq!(
            DevPass.fetch(&scope.context()).await.unwrap(),
            Reading::new(
                Some("Pay As You Go".into()),
                vec![lines::dollar_value("Total Usage", 3.0)]
            )
        );
    }

    #[tokio::test]
    async fn http_errors_and_unreadable_amounts_keep_their_categories() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{}", ErrorCategory::Decoding),
            (
                200,
                r#"{"data":{"devPlan":"none","usage":"NaN"}}"#,
                ErrorCategory::Decoding,
            ),
        ] {
            let http = Scripted::new().on("GET", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
            assert_eq!(
                DevPass.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }
}
