//! Notion AI: the share of a Notion workspace's AI allowance used in its current window and in its
//! billing period.
//!
//! Nothing is read from disk on Windows, macOS or Linux, browser cookie jars are never read and
//! nothing is renewed: the value of Notion's `token_v2` cookie and the workspace ID are pasted
//! into Accounts. A refresh sends one request,
//! `POST https://app.notion.com/api/v3/getCreditRateLimitStatus`, with `Cookie: token_v2=<value>`,
//! Notion's `Origin` and `Referer`, and `{"spaceId": <workspace ID>}` as its JSON body. The
//! answer's `window` is the Session meter, resetting `resetsInSeconds` from now, and its
//! `billingPeriodWindow` the Monthly meter, resetting at `periodEndMs`. A workspace whose `status`
//! is `not_applicable` has no tracked allowance.

use async_trait::async_trait;
use serde_json::json;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Notion;

const URL: &str = "https://app.notion.com/api/v3/getCreditRateLimitStatus";

#[async_trait]
impl Service for Notion {
    fn id(&self) -> &'static str {
        "notion"
    }

    fn name(&self) -> &'static str {
        "Notion AI"
    }

    fn key_label(&self) -> &'static str {
        "Session cookie (token_v2)"
    }

    fn key_format(&self) -> KeyFormat {
        KeyFormat::Cookie
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "https://app.notion.com",
            fields: &[("spaceId", "Workspace ID")],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [("session", "Session"), ("monthly", "Monthly")]
            .into_iter()
            .map(|(id, title)| {
                WidgetDescriptor::percent(
                    format!("{}.{id}", provider.id),
                    provider,
                    title,
                    None,
                    None,
                )
                .exporting_progress(id, "percent")
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let token = context
            .secret
            .key()
            .filter(|token| {
                !token
                    .chars()
                    .any(|character| character.is_control() || character == ';')
            })
            .ok_or_else(|| http::invalid("The Notion token_v2 cookie is missing or invalid."))?;
        let space = context
            .secret
            .str("/spaceId")
            .ok_or_else(|| http::invalid("Enter the Notion workspace ID."))?;
        let body = http::json(
            context.http,
            HttpRequest::post(URL)
                .header("Cookie", format!("token_v2={token}"))
                .header("Origin", "https://app.notion.com")
                .header("Referer", "https://app.notion.com/")
                .json_body(&json!({"spaceId":space})),
            "Notion AI",
        )
        .await?;
        if value::text(&body, "/status") == Some("not_applicable") {
            return Err(http::not_available(
                "This Notion workspace has no tracked AI allowance.",
            ));
        }
        let mut meters = vec![];
        for (path, title) in [("/window", "Session"), ("/billingPeriodWindow", "Monthly")] {
            if let Some(window) = body.pointer(path)
                && let (Some(used), Some(limit)) = (
                    value::number(window, "/used"),
                    value::number(window, "/limit"),
                )
            {
                let reset = if path == "/window" {
                    value::number(&body, "/resetsInSeconds")
                        .filter(|seconds| *seconds >= 0.0 && *seconds <= 31_536_000.0)
                        .and_then(|seconds| {
                            context
                                .now
                                .checked_add_signed(chrono::Duration::milliseconds(
                                    (seconds * 1000.0) as i64,
                                ))
                        })
                } else {
                    value::time(window, "/periodEndMs")
                };
                if let Some(line) = lines::percent_of(title, used, limit, reset, None) {
                    meters.push(line);
                }
            }
        }
        if meters.is_empty() {
            return Err(http::decoding("Notion AI"));
        }
        Ok(Reading::new(None, meters))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Utc;
    use uc_core::ErrorCategory;

    const RATE_LIMIT_STATUS: &str = r#"{"status":"active","window":{"used":25,"limit":100,"window":"5h"},"resetsInSeconds":3600,"billingPeriodWindow":{"used":250,"limit":1000,"periodEndMs":1790812800000}}"#;

    #[tokio::test]
    async fn reads_the_session_and_monthly_windows_of_the_saved_workspace() {
        let http = Scripted::new().on("POST", URL, 200, RATE_LIMIT_STATUS);
        let now = Utc::now();
        let scope = context_at(&http, json!({"apiKey":"test","spaceId":"space-1"}), now);
        let reading = Notion.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, None);
        assert_eq!(
            reading.lines,
            vec![
                lines::percent(
                    "Session",
                    25.0,
                    Some(now + chrono::Duration::hours(1)),
                    None
                ),
                lines::percent(
                    "Monthly",
                    25.0,
                    value::as_time(&json!(1790812800000i64)),
                    None
                )
            ]
        );
        assert_eq!(header(&http.requests()[0], "Cookie"), Some("token_v2=test"));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(http.requests()[0].body.as_ref().unwrap())
                .unwrap(),
            json!({"spaceId":"space-1"})
        );
    }

    #[tokio::test]
    async fn failed_unreadable_or_untracked_answers_keep_their_categories() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{}", ErrorCategory::Decoding),
            (
                200,
                r#"{"status":"not_applicable"}"#,
                ErrorCategory::NotAvailable,
            ),
        ] {
            let http = Scripted::new().on("POST", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"test","spaceId":"s"}), Utc::now());
            assert_eq!(
                Notion.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }
}
