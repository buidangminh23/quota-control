//! Manus: the share of a Manus account's monthly and refresh credits already used, its credit
//! balance, and its free, add-on and event credits when Manus lists them.
//!
//! Nothing is read from disk or from a browser on Windows, macOS or Linux: the card takes the value
//! of the `session_id` cookie pasted into Quota Control; a value holding `;`, spaces or control
//! characters is refused, so a whole cookie list cannot be pasted. A refresh sends one request,
//! `POST https://api.manus.im/user.v1.UserService/GetAvailableCredits` with the session as a bearer
//! token, an empty JSON body, an Origin and Referer on `https://manus.im`,
//! `Connect-Protocol-Version: 1` and a `QuotaControl` user agent. The credits may sit under `data`,
//! `result`, `response` or `availableCredits`. A numeric `nextRefreshTime` counts seconds from
//! 1 January 2001, and a text one is an RFC 3339 time.

use async_trait::async_trait;
use serde_json::{Value, json};
use uc_core::{HttpRequest, MetricKind, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Manus;

const NAME: &str = "Manus";
const URL: &str = "https://api.manus.im/user.v1.UserService/GetAvailableCredits";

#[async_trait]
impl Service for Manus {
    fn id(&self) -> &'static str {
        "manus"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "https://manus.im",
            fields: &[],
        })
    }

    fn key_label(&self) -> &'static str {
        "Session cookie (session_id)"
    }

    fn key_format(&self) -> KeyFormat {
        KeyFormat::Cookie
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::percent(
                format!("{}.monthly", provider.id),
                provider,
                "Monthly",
                None,
                None,
            )
            .exporting_progress("monthly", "percent"),
            WidgetDescriptor::percent(
                format!("{}.refresh", provider.id),
                provider,
                "Refresh",
                None,
                None,
            )
            .exporting_progress("refresh", "percent"),
            WidgetDescriptor::values(
                format!("{}.balance", provider.id),
                provider,
                "Balance",
                None,
                Some(MetricKind::Count),
                Some("credits"),
                true,
                None,
                false,
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .filter(|session| {
                !session.contains(';')
                    && !session
                        .bytes()
                        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
            })
            .ok_or_else(|| http::invalid("Paste the Manus session_id cookie value."))?;
        let root = http::json(
            context.http,
            HttpRequest::post(URL)
                .bearer(key)
                .json_body(&json!({}))
                .header("Origin", "https://manus.im")
                .header("Referer", "https://manus.im/")
                .header("Connect-Protocol-Version", "1")
                .header("User-Agent", "QuotaControl"),
            NAME,
        )
        .await?;
        let data = ["data", "result", "response", "availableCredits"]
            .iter()
            .find_map(|wrapper| root.get(wrapper).filter(|credits| !credits.is_null()))
            .unwrap_or(&root);
        let keys = [
            "totalCredits",
            "freeCredits",
            "periodicCredits",
            "addonCredits",
            "refreshCredits",
            "maxRefreshCredits",
            "proMonthlyCredits",
            "eventCredits",
        ];
        if !data.is_object() || !keys.iter().any(|field| data.get(field).is_some()) {
            return Err(http::decoding(NAME));
        }
        let number = |field: &str| -> Result<f64, SimpleProviderError> {
            match data.get(field) {
                None | Some(Value::Null) => Ok(0.0),
                Some(raw) => value::number(raw, "")
                    .filter(|credits| *credits >= 0.0)
                    .ok_or_else(|| http::decoding(NAME)),
            }
        };
        let total = number("totalCredits")?;
        let monthly = number("proMonthlyCredits")?;
        let periodic = number("periodicCredits")?;
        let refresh = number("refreshCredits")?;
        let max = number("maxRefreshCredits")?;
        let reset = match &data["nextRefreshTime"] {
            Value::Number(seconds) => seconds
                .as_f64()
                .filter(|seconds| {
                    seconds.is_finite() && *seconds >= 0.0 && *seconds < 63_000_000_000.0
                })
                .and_then(|seconds| {
                    chrono::DateTime::from_timestamp((seconds + 978_307_200.0) as i64, 0)
                }),
            Value::String(text) => chrono::DateTime::parse_from_rfc3339(text)
                .ok()
                .map(|time| time.with_timezone(&chrono::Utc)),
            _ => None,
        };
        let mut rows = Vec::new();
        if let Some(row) = lines::percent_of(
            "Monthly",
            monthly - periodic,
            monthly,
            None,
            Some(lines::MONTH_MS),
        ) {
            rows.push(row);
        }
        if let Some(row) = lines::percent_of("Refresh", max - refresh, max, reset, None) {
            rows.push(row);
        }
        if data.get("totalCredits").is_some() {
            rows.push(lines::count_value("Balance", total, "credits"));
        }
        for (field, label) in [
            ("freeCredits", "Free"),
            ("addonCredits", "Add-on"),
            ("eventCredits", "Event"),
        ] {
            if data.get(field).is_some() {
                rows.push(lines::count_value(label, number(field)?, "credits"));
            }
        }
        if rows.is_empty() {
            return Err(http::not_available(
                "Manus did not include a credit balance or quota.",
            ));
        }
        Ok(Reading::new(None, rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use uc_core::ErrorCategory;

    #[tokio::test]
    async fn credits_are_read_with_a_bearer_session_and_the_refresh_time_counts_from_2001() {
        let body = r#"{"data":{"totalCredits":"80","proMonthlyCredits":100,"periodicCredits":40,"maxRefreshCredits":20,"refreshCredits":5,"nextRefreshTime":800000000}}"#;
        let http = Scripted::new().on("POST", URL, 200, body);
        let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
        let reading = Manus.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![
                lines::percent("Monthly", 60.0, None, Some(lines::MONTH_MS)),
                lines::percent(
                    "Refresh",
                    75.0,
                    chrono::DateTime::from_timestamp(1_778_307_200, 0),
                    None
                ),
                lines::count_value("Balance", 80.0, "credits")
            ]
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            header(&requests[0], "authorization"),
            Some("Bearer fixture")
        );
        assert_eq!(header(&requests[0], "cookie"), None);
        assert_eq!(requests[0].body, Some(b"{}".to_vec()));
    }

    #[tokio::test]
    async fn errors_keep_their_categories_and_a_pasted_cookie_list_is_refused() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{}", ErrorCategory::Decoding),
            (200, r#"{"totalCredits":"bad"}"#, ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("POST", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
            assert_eq!(
                Manus.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
        let http = Scripted::new();
        let scope = context_at(
            &http,
            json!({"apiKey":"session_id=x; other=y"}),
            chrono::Utc::now(),
        );
        assert_eq!(
            Manus.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
