//! Muse Code: the session and weekly windows of a Muse Code subscription.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key is a Muse Code device-login token
//! starting with `dca:`, from the `MUSE_DEVICE_TOKEN` environment variable or saved in Quota
//! Control, and any other key sends nothing. A refresh sends one request,
//! `POST https://api.meta.ai/muse-code/key` with the token as a bearer token, an empty JSON body,
//! `x-api-version: 1.0.0` and the `QuotaControl` user agent. The card shows the subscription tier
//! as the plan and the percent used of the session window, whose length the answer gives in
//! minutes, and of the weekly window, each with its reset time. A login that still needs a payment
//! method, has no active subscription or comes without quota shows that instead. There is no
//! fallback to browser team quota, no login flow, no credential refresh, no write to local
//! credentials and no inference request.

use async_trait::async_trait;
use serde_json::json;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Muse;

const NAME: &str = "Muse Code";
const URL: &str = "https://api.meta.ai/muse-code/key";
const MISSING: &str = "Muse Code requires a device-code login token starting with dca:.";
const IDLE: &str =
    "Muse Code did not include quota in this login response. Browser team quota is not supported.";

#[async_trait]
impl Service for Muse {
    fn id(&self) -> &'static str {
        "muse"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["MUSE_DEVICE_TOKEN"],
            url: "https://dev.meta.ai",
            fields: &[],
        })
    }

    fn key_label(&self) -> &'static str {
        "Session token"
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [("session", "Session"), ("weekly", "Weekly")]
            .iter()
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
            .filter(|candidate| {
                candidate.starts_with("dca:")
                    && candidate.len() > 4
                    && !candidate
                        .bytes()
                        .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
            })
            .ok_or_else(|| http::invalid(MISSING))?;
        let data = http::json(
            context.http,
            HttpRequest::post(URL)
                .bearer(token)
                .json_body(&json!({}))
                .header("x-api-version", "1.0.0")
                .header("User-Agent", "QuotaControl"),
            NAME,
        )
        .await?;
        for key in ["require_payment", "is_subs_active"] {
            if !data[key].is_null() && !data[key].is_boolean() {
                return Err(http::decoding(NAME));
            }
        }
        if data["require_payment"] == true {
            return Err(http::not_available(
                "Muse Code requires a payment method. Finish billing at https://dev.meta.ai",
            ));
        }
        if data["is_subs_active"] != true {
            return Err(http::not_available(
                "No Muse Code subscription is active on this login.",
            ));
        }
        let plan = if data["subs_tier_name"].is_null() {
            None
        } else {
            Some(
                data["subs_tier_name"]
                    .as_str()
                    .ok_or_else(|| http::decoding(NAME))?
                    .trim()
                    .to_string(),
            )
        }
        .filter(|name| !name.is_empty());
        if data["subs_usage"].is_null() {
            return Err(http::not_available(IDLE));
        }
        let usage = &data["subs_usage"];
        let minutes = usage["window"]["window_duration_mins"]
            .as_f64()
            .filter(|duration| {
                duration.is_finite()
                    && duration.round() >= 1.0
                    && *duration <= (i64::MAX / 60_000) as f64
            })
            .ok_or_else(|| http::decoding(NAME))?
            .round() as i64;
        let mut rows = Vec::new();
        for (key, title, period) in [
            ("window", "Session", minutes * 60_000),
            ("weekly", "Weekly", lines::WEEK_MS),
        ] {
            let window = &usage[key];
            let percent = window["used_percent"]
                .as_f64()
                .filter(|number| number.is_finite())
                .ok_or_else(|| http::decoding(NAME))?;
            let reset = if window["resets_at"].is_null() {
                None
            } else {
                let seconds = window["resets_at"]
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| http::decoding(NAME))?;
                if seconds > 0.0 && seconds <= 64_092_211_200.0 {
                    value::time(window, "/resets_at")
                } else {
                    None
                }
            };
            rows.push(lines::percent(title, percent, reset, Some(period)));
        }
        Ok(Reading::new(plan, rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use uc_core::ErrorCategory;

    const ACTIVE_SUBSCRIPTION: &str = r#"{"is_subs_active":true,"subs_tier_name":"Pro","subs_usage":{"window":{"window_duration_mins":300,"used_percent":25,"resets_at":1790000000},"weekly":{"used_percent":70}}}"#;

    #[tokio::test]
    async fn an_active_subscription_reads_its_plan_and_windows_in_one_request_to_muse() {
        let http = Scripted::new().on("POST", URL, 200, ACTIVE_SUBSCRIPTION);
        let scope = context_at(&http, json!({"apiKey":"dca:fixture"}), chrono::Utc::now());
        let reading = Muse.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, Some("Pro".into()));
        assert_eq!(
            reading.lines[1],
            lines::percent("Weekly", 70.0, None, Some(lines::WEEK_MS))
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, URL);
        assert_eq!(
            header(&requests[0], "authorization"),
            Some("Bearer dca:fixture")
        );
        assert_eq!(header(&requests[0], "x-api-version"), Some("1.0.0"));
    }

    #[tokio::test]
    async fn unavailable_or_failed_reads_keep_their_category_and_non_dca_keys_send_nothing() {
        for (status, body, category) in [
            (
                200,
                r#"{"is_subs_active":true}"#,
                ErrorCategory::NotAvailable,
            ),
            (
                200,
                r#"{"require_payment":true}"#,
                ErrorCategory::NotAvailable,
            ),
            (
                200,
                r#"{"is_subs_active":false}"#,
                ErrorCategory::NotAvailable,
            ),
            (200, r#"{"is_subs_active":"true"}"#, ErrorCategory::Decoding),
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("POST", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"dca:fixture"}), chrono::Utc::now());
            assert_eq!(
                Muse.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
            assert_eq!(http.requests().len(), 1);
        }
        let http = Scripted::new();
        let scope = context_at(
            &http,
            json!({"apiKey":"not-device-token"}),
            chrono::Utc::now(),
        );
        assert!(Muse.fetch(&scope.context()).await.is_err());
        assert!(http.requests().is_empty());
    }
}
