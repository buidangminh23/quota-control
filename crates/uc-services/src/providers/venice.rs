//! Venice: the dollar balance and the DIEM credits of a Venice API key.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the `VENICE_API_KEY`
//! environment variable or from a key saved in Quota Control. A refresh sends one request,
//! `GET https://api.venice.ai/api/v1/billing/balance` with the key as a bearer token, and shows the
//! dollar balance and the DIEM credits: the DIEM spent against the epoch's allocation when Venice
//! reports one (`diemEpochAllocation`), otherwise the DIEM left. A warning says when Venice reports
//! that the balance cannot pay for API calls (`canConsume` is false).

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Venice;

const NAME: &str = "Venice";
const URL: &str = "https://api.venice.ai/api/v1/billing/balance";

#[async_trait]
impl Service for Venice {
    fn id(&self) -> &'static str {
        "venice"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["VENICE_API_KEY"],
            url: "https://venice.ai/settings/api",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::dollar_balance(
                format!("{}.balance", provider.id),
                provider,
                "Balance",
                None,
                "left",
            ),
            WidgetDescriptor::bounded_count(
                format!("{}.credits", provider.id),
                provider,
                "Credits",
                None,
                0.0,
                "DIEM",
                None,
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Venice API key is missing."))?;
        let body = http::json(context.http, HttpRequest::get(URL).bearer(key), NAME).await?;
        parse(&body)
    }
}

/// The Balance and Credits rows of a balance answer, with a warning when it cannot pay for calls.
fn parse(body: &Value) -> Result<Reading, SimpleProviderError> {
    let can_consume = body
        .get("canConsume")
        .and_then(Value::as_bool)
        .ok_or_else(|| http::decoding(NAME))?;
    if !body.get("balances").is_some_and(Value::is_object) {
        return Err(http::decoding(NAME));
    }
    let usd = value::number(body, "/balances/usd");
    let diem = value::number(body, "/balances/diem");
    let allocation = value::number(body, "/diemEpochAllocation");
    let mut rows = Vec::new();
    if let Some(usd) = usd {
        rows.push(lines::dollar_value("Balance", usd));
    }
    if let Some(diem) = diem {
        if let Some(limit) = allocation.filter(|allocation| *allocation > 0.0) {
            rows.push(lines::count(
                "Credits",
                (limit - diem).max(0.0),
                limit,
                "DIEM",
                None,
                None,
            ));
        } else {
            rows.push(lines::count_value("Credits", diem, "DIEM"));
        }
    }
    if rows.is_empty() {
        return Err(http::decoding(NAME));
    }
    Ok(Reading::new(None, rows).with_warning(
        (!can_consume).then(|| "The Venice balance is unavailable for API calls.".into()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const BALANCE_BODY: &str = r###"{"canConsume":true,"consumptionCurrency":"DIEM","balances":{"usd":"12.5","diem":80},"diemEpochAllocation":100}"###;

    #[tokio::test]
    async fn one_bearer_request_shows_the_dollar_balance_and_the_diem_spent_this_epoch() {
        let http = Scripted::new().on("GET", URL, 200, BALANCE_BODY);
        let now = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        let scope = context_at(&http, json!({"apiKey":"test"}), now);
        let reading = Venice.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                None,
                vec![
                    lines::dollar_value("Balance", 12.5),
                    lines::count("Credits", 20.0, 100.0, "DIEM", None, None)
                ]
            )
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, URL);
        assert_eq!(header(&requests[0], "Authorization"), Some("Bearer test"));
        assert_eq!(requests[0].body, None);
    }

    #[tokio::test]
    async fn failed_or_unreadable_answers_keep_their_categories_without_echoing_the_body() {
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
            let error = Venice.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn missing_secret_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        assert_eq!(
            Venice.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::AuthInvalid
        );
        assert!(http.requests().is_empty());
    }
}
