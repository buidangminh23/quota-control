//! ZoomMate: the credits used in the current ZoomMate cycle.
//!
//! Nothing is read from disk or the environment on Windows, macOS or Linux, and no browser cookie
//! is read: the key is the bearer token of a ZoomMate session, the `Authorization` header's value
//! with or without `Bearer `, saved in Quota Control. The token is never renewed, so one whose JWT
//! expiry has passed is reported as expired without a request. Otherwise a refresh sends one
//! request, `GET https://ai.zoom.us/ai-computer/api/v1/credits/status`, with the token as a bearer
//! token and `Referer: https://zoommate.zoom.us/`, and shows the credits used against the budget
//! cap, renewing when the cycle ends, or the credits used alone under an Unlimited plan.

use async_trait::async_trait;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, jwt, lines, value};

pub(crate) struct ZoomMate;

const URL: &str = "https://ai.zoom.us/ai-computer/api/v1/credits/status";

#[async_trait]
impl Service for ZoomMate {
    fn id(&self) -> &'static str {
        "zoommate"
    }

    fn name(&self) -> &'static str {
        "ZoomMate"
    }

    fn key_label(&self) -> &'static str {
        "Session bearer token (Authorization)"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "https://zoommate.zoom.us",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![WidgetDescriptor::bounded_count(
            format!("{}.credits", provider.id),
            provider,
            "Credits",
            None,
            0.0,
            "credits",
            None,
        )]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let raw = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The ZoomMate session token is missing."))?;
        let token = raw.strip_prefix("Bearer ").unwrap_or(raw);
        if jwt::expires_at(token).is_some_and(|expires| expires <= context.now) {
            return Err(http::expired(
                "The ZoomMate session expired. Sign in and paste a fresh bearer token.",
            ));
        }
        let body = http::json(
            context.http,
            HttpRequest::get(URL)
                .bearer(token)
                .header("Referer", "https://zoommate.zoom.us/"),
            "ZoomMate",
        )
        .await?;
        let data = body
            .pointer("/data/credit_status")
            .ok_or_else(|| http::decoding("ZoomMate"))?;
        let used = value::number(data, "/used_credit").ok_or_else(|| http::decoding("ZoomMate"))?;
        let unlimited = value::flag(data, "/is_unlimited") == Some(true);
        let line = if unlimited {
            lines::count_value("Credits", used, "credits")
        } else {
            lines::count(
                "Credits",
                used,
                value::number(data, "/budget_cap").ok_or_else(|| http::decoding("ZoomMate"))?,
                "credits",
                value::time(data, "/cycle_end_date"),
                None,
            )
        };
        Ok(Reading::new(
            unlimited.then(|| "Unlimited".into()),
            vec![line],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Utc;
    use serde_json::json;
    use uc_core::ErrorCategory;

    const STATUS: &str = r#"{"data":{"credit_status":{"budget_cap":100,"used_credit":25,"remaining_credit":75,"cycle_end_date":1790812800000,"is_unlimited":false}}}"#;

    #[tokio::test]
    async fn shows_the_credits_used_against_the_cap_and_strips_a_pasted_bearer_prefix() {
        let http = Scripted::new().on("GET", URL, 200, STATUS);
        let scope = context_at(&http, json!({"apiKey": "Bearer test"}), Utc::now());
        let reading = ZoomMate.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, None);
        assert_eq!(
            reading.lines,
            vec![lines::count(
                "Credits",
                25.0,
                100.0,
                "credits",
                value::as_time(&json!(1790812800000i64)),
                None
            )]
        );
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some("Bearer test")
        );
    }

    #[tokio::test]
    async fn a_refused_limited_or_unreadable_answer_keeps_its_error_category() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (200, ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", URL, status, "{}");
            let scope = context_at(&http, json!({"apiKey": "test"}), Utc::now());
            assert_eq!(
                ZoomMate.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }
}
