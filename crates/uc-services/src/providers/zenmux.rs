//! ZenMux: the Session (5-hour) and Weekly flow quotas of a ZenMux subscription, read with a
//! Management API key. Nothing is read from disk on Windows, macOS or Linux: the key comes from
//! the `ZENMUX_MANAGEMENT_API_KEY` environment variable or from a key saved in Quota Control.
//!
//! A refresh sends one request, `GET https://zenmux.ai/api/v1/management/subscription/detail`,
//! with the key as a bearer token. The answer's `quota_5_hour` and `quota_7_day` give the flows
//! used, the flow limit and the time each window resets, `plan.tier` names the plan and
//! `plan.expires_at` is when the current subscription period ends
//! (https://zenmux.ai/docs/api/platform/subscription-detail.html).

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, PlanTerm, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct ZenMux;

const NAME: &str = "ZenMux";
const URL: &str = "https://zenmux.ai/api/v1/management/subscription/detail";

#[async_trait]
impl Service for ZenMux {
    fn id(&self) -> &'static str {
        "zenmux"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn key_label(&self) -> &'static str {
        "Management API key"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["ZENMUX_MANAGEMENT_API_KEY"],
            url: "https://zenmux.ai",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [("session", "Session"), ("weekly", "Weekly")]
            .into_iter()
            .map(|(id, title)| {
                WidgetDescriptor::bounded_count(
                    format!("{}.{id}", provider.id),
                    provider,
                    title,
                    None,
                    0.0,
                    "flows",
                    None,
                )
                .exporting_progress(id, "flows")
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The ZenMux Management API key is missing."))?;
        let body = http::json(context.http, HttpRequest::get(URL).bearer(key), NAME).await?;
        parse(&body)
    }
}

/// The Session and Weekly flow rows of a subscription answer, and its plan name.
fn parse(body: &Value) -> Result<Reading, SimpleProviderError> {
    if body.get("success") != Some(&Value::Bool(true)) {
        return Err(http::decoding(NAME));
    }
    let data = body.get("data").ok_or_else(|| http::decoding(NAME))?;
    let mut rows = Vec::new();
    for (path, label, period) in [
        ("/quota_5_hour", "Session", 5 * lines::HOUR_MS),
        ("/quota_7_day", "Weekly", lines::WEEK_MS),
    ] {
        let quota = data.pointer(path).ok_or_else(|| http::decoding(NAME))?;
        let used = value::number(quota, "/used_flows").ok_or_else(|| http::decoding(NAME))?;
        let total = value::number(quota, "/max_flows").ok_or_else(|| http::decoding(NAME))?;
        if used < 0.0 || total < 0.0 {
            return Err(http::decoding(NAME));
        }
        rows.push(lines::count(
            label,
            used,
            total,
            "flows",
            value::time(quota, "/resets_at"),
            Some(period),
        ));
    }
    let plan = value::text(data, "/plan/tier").and_then(lines::plan_name);
    let term = value::time(data, "/plan/expires_at").map(|ends_at| PlanTerm::Stated {
        ends_at,
        checked_at: None,
    });
    Ok(Reading::new(plan, rows).with_plan_term(term))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const PRO_QUOTAS: &str = r###"{"success":true,"data":{"plan":{"tier":"pro","amount_usd":20,"interval":"month","expires_at":"2026-10-12T08:26:56.000Z"},"account_status":"healthy","quota_5_hour":{"used_flows":20,"max_flows":100,"remaining_flows":80,"usage_percentage":0.2,"resets_at":"2026-09-27T15:00:00Z"},"quota_7_day":{"used_flows":50,"max_flows":1000,"remaining_flows":950,"usage_percentage":0.05,"resets_at":"2026-10-01T00:00:00Z"}}}"###;

    #[tokio::test]
    async fn the_session_and_weekly_flow_quotas_come_from_one_authorized_get() {
        let http = Scripted::new().on("GET", URL, 200, PRO_QUOTAS);
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey":"test"}), now);
        let reading = ZenMux.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::count(
                        "Session",
                        20.0,
                        100.0,
                        "flows",
                        Some(Utc.with_ymd_and_hms(2026, 9, 27, 15, 0, 0).unwrap()),
                        Some(5 * lines::HOUR_MS)
                    ),
                    lines::count(
                        "Weekly",
                        50.0,
                        1000.0,
                        "flows",
                        Some(Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()),
                        Some(lines::WEEK_MS)
                    )
                ]
            )
            .with_plan_term(Some(PlanTerm::Stated {
                ends_at: Utc.with_ymd_and_hms(2026, 10, 12, 8, 26, 56).unwrap(),
                checked_at: None,
            }))
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, URL);
        assert_eq!(header(&requests[0], "Authorization"), Some("Bearer test"));
        assert_eq!(requests[0].body, None);
    }

    #[tokio::test]
    async fn failed_or_unreadable_answers_keep_their_categories_without_the_body() {
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
            let error = ZenMux.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            ZenMux.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
