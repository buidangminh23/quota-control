//! LongCat: the tokens used of an active LongCat token pack, or else of the account's token quota
//! with a row for each model it lists.
//!
//! Nothing is read from disk or from a browser on Windows, macOS or Linux: the card takes the
//! whole Cookie header of a signed-in LongCat quota request, pasted into Quota Control. A refresh
//! first sends `POST https://longcat.chat/api/pay/quota/metering/token-packs/summary` with that
//! header and an empty JSON body. When the summary holds a `currentLot` whose status is `ACTIVE`,
//! its `consumedToken` of `totalToken` is the Tokens row and nothing more is sent. Otherwise, and
//! also when the summary fails with a status other than 401, 403 or 429, it sends
//! `GET https://longcat.chat/api/lc-platform/v1/tokenUsage` with the same header and shows the
//! `usedToken` of `totalToken` it answers, then a row for each model under `extData`. Both answers
//! carry a `code` beside their `data`, and a code of 401 or 403 means the session expired. Neither
//! request runs a model.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct LongCat;

const BASE: &str = "https://longcat.chat";
const SUMMARY: &str = "https://longcat.chat/api/pay/quota/metering/token-packs/summary";
const USAGE: &str = "https://longcat.chat/api/lc-platform/v1/tokenUsage";

#[async_trait]
impl Service for LongCat {
    fn id(&self) -> &'static str {
        "longcat"
    }

    fn name(&self) -> &'static str {
        "LongCat"
    }

    fn key_label(&self) -> &'static str {
        "Cookie header"
    }

    fn key_format(&self) -> KeyFormat {
        KeyFormat::CookieHeader
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: BASE,
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![WidgetDescriptor::bounded_count(
            format!("{}.tokens", provider.id),
            provider,
            "Tokens",
            None,
            0.0,
            "tokens",
            None,
        )]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let cookie = context
            .secret
            .key()
            .filter(|cookie| cookie.contains('=') && !cookie.contains(['\r', '\n']))
            .ok_or_else(|| {
                http::invalid("Paste the LongCat Cookie header from a signed-in quota request.")
            })?;
        let request = HttpRequest::post(SUMMARY)
            .header("Cookie", cookie)
            .json_body(&serde_json::json!({}));
        let response = http::send(context.http, request, "LongCat").await?;
        let summary = if response.is_success() {
            Some(envelope(http::parse(&response, "LongCat")?)?)
        } else if matches!(response.status, 401 | 403 | 429) {
            return Err(http::status_error(&response, "LongCat"));
        } else {
            None
        };
        if let Some(lot) = summary.as_ref().and_then(|data| data.get("currentLot"))
            && value::text(lot, "/status") == Some("ACTIVE")
        {
            let total = value::number(lot, "/totalToken")
                .filter(|total| *total > 0.0)
                .ok_or_else(|| http::decoding("LongCat"))?;
            let used =
                value::number(lot, "/consumedToken").ok_or_else(|| http::decoding("LongCat"))?;
            return Ok(Reading::new(
                None,
                vec![lines::count("Tokens", used, total, "tokens", None, None)],
            ));
        }
        let data = envelope(
            http::json(
                context.http,
                HttpRequest::get(USAGE).header("Cookie", cookie),
                "LongCat",
            )
            .await?,
        )?;
        let usage = data.get("usage").unwrap_or(&data);
        let total = value::number(usage, "/totalToken").ok_or_else(|| http::decoding("LongCat"))?;
        let used = value::number(usage, "/usedToken").ok_or_else(|| http::decoding("LongCat"))?;
        let mut rows = vec![lines::count("Tokens", used, total, "tokens", None, None)];
        if let Some(models) = data.get("extData").and_then(Value::as_object) {
            for (name, model) in models {
                if let (Some(total), Some(used)) = (
                    value::number(model, "/totalToken"),
                    value::number(model, "/usedToken"),
                ) {
                    rows.push(lines::count(name, used, total, "tokens", None, None));
                }
            }
        }
        Ok(Reading::new(None, rows))
    }
}

/// The `data` object of a LongCat answer whose `code` is 0. A code of 401 or 403 means the session
/// cookies expired; any other code, or a missing `data` object, cannot be read.
fn envelope(body: Value) -> Result<Value, SimpleProviderError> {
    match value::number(&body, "/code") {
        Some(0.0) => body
            .get("data")
            .filter(|data| data.is_object())
            .cloned()
            .ok_or_else(|| http::decoding("LongCat")),
        Some(401.0 | 403.0) => Err(http::expired(
            "The LongCat session expired. Sign in and paste fresh session cookies.",
        )),
        _ => Err(http::decoding("LongCat")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Utc;
    use serde_json::json;
    use uc_core::ErrorCategory;

    #[tokio::test]
    async fn without_an_active_pack_the_quota_shows_with_a_row_per_model() {
        let usage = r#"{"code":0,"data":{"usage":{"totalToken":500000,"usedToken":120000},"extData":{"LongCat-Flash-Lite":{"totalToken":50000000,"usedToken":100}}}}"#;
        let http = Scripted::new()
            .on("POST", SUMMARY, 200, r#"{"code":0,"data":{}}"#)
            .on("GET", USAGE, 200, usage);
        let scope = context_at(&http, json!({"apiKey":"session=test"}), Utc::now());
        let reading = LongCat.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![
                lines::count("Tokens", 120000.0, 500000.0, "tokens", None, None),
                lines::count(
                    "LongCat-Flash-Lite",
                    100.0,
                    50000000.0,
                    "tokens",
                    None,
                    None
                )
            ]
        );
        assert_eq!(reading.plan, None);
        assert_eq!(header(&http.requests()[0], "Cookie"), Some("session=test"));
        assert_eq!(http.requests().len(), 2);
    }

    #[tokio::test]
    async fn expired_sessions_rate_limits_and_unreadable_summaries_keep_their_categories() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, r#"{"code":401}"#, ErrorCategory::AuthExpired),
            (200, "{}", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("POST", SUMMARY, status, body);
            let scope = context_at(&http, json!({"apiKey":"session=test"}), Utc::now());
            assert_eq!(
                LongCat.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }

    #[tokio::test]
    async fn an_active_token_pack_is_read_from_the_summary_alone() {
        let summary = r#"{"code":0,"data":{"currentLot":{"status":"ACTIVE","totalToken":1000,"consumedToken":250}}}"#;
        let http = Scripted::new().on("POST", SUMMARY, 200, summary);
        let scope = context_at(&http, json!({"apiKey":"session=test"}), Utc::now());
        assert_eq!(
            LongCat.fetch(&scope.context()).await.unwrap().lines,
            vec![lines::count("Tokens", 250.0, 1000.0, "tokens", None, None)]
        );
        assert_eq!(http.requests().len(), 1);
    }
}
