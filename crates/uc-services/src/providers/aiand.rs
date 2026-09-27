//! ai&: what an ai& API key has spent over the last 30 days.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the `AIAND_API_KEY`
//! environment variable or from a key saved in Quota Control. A refresh pages through
//! `GET https://api.aiand.com/logs?range=30days&limit=100` with the key as a bearer token, passing
//! the answer's `next_after` and `next_after_id` cursors back as `after` and `after_id` while
//! `has_more` is true, and sends at most four requests. The `cost` of every log in the first
//! currency seen is added up and shown in dollars for `USD`, or as an amount of that currency;
//! logs in another currency are left out. When `has_more` is still true after four requests, or
//! the next cursor is missing or repeats, the total carries a warning that it is incomplete.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct AiAnd;

const NAME: &str = "ai&";
const URL: &str = "https://api.aiand.com/logs?range=30days&limit=100";
const TITLE: &str = "Last 30 Days";
const PARTIAL: &str =
    "The last 30 days are only partially available within the four-request limit.";

#[async_trait]
impl Service for AiAnd {
    fn id(&self) -> &'static str {
        "aiand"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["AIAND_API_KEY"],
            url: "https://console.aiand.com",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![WidgetDescriptor::values(
            format!("{}.month", provider.id),
            provider,
            TITLE,
            None,
            None,
            None,
            true,
            None,
            false,
        )]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The ai& API key is missing."))?;
        let mut currency = None::<String>;
        let (mut sum, mut correction) = (0.0_f64, 0.0_f64);
        let mut cursor = None::<(String, String)>;
        let mut seen = std::collections::HashSet::new();
        let mut complete = false;
        for _ in 0..4 {
            let mut page_url = url::Url::parse(URL).map_err(|_| http::decoding(NAME))?;
            if let Some((after, id)) = &cursor {
                page_url
                    .query_pairs_mut()
                    .append_pair("after", after)
                    .append_pair("after_id", id);
            }
            let body = http::json(
                context.http,
                HttpRequest::get(page_url.as_str())
                    .bearer(key)
                    .header("Accept", "application/json"),
                NAME,
            )
            .await?;
            let rows = body
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| http::decoding(NAME))?;
            if body
                .get("has_more")
                .is_some_and(|has_more| !has_more.is_null() && !has_more.is_boolean())
            {
                return Err(http::decoding(NAME));
            }
            let after = optional_text(&body, "next_after")?;
            let id = optional_text(&body, "next_after_id")?;
            for row in rows {
                if !row.is_object() {
                    return Err(http::decoding(NAME));
                }
                let cost = optional_text(row, "cost")?.and_then(decimal);
                let code = optional_text(row, "currency")?
                    .map(|text| text.trim().to_ascii_uppercase())
                    .filter(|code| !code.is_empty());
                let (Some(cost), Some(code)) = (cost, code) else {
                    continue;
                };
                if currency.get_or_insert_with(|| code.clone()) != &code {
                    continue;
                }
                let adjusted = cost - correction;
                let next = sum + adjusted;
                correction = (next - sum) - adjusted;
                sum = next;
                if !sum.is_finite() {
                    return Err(http::decoding(NAME));
                }
            }
            if body.get("has_more").and_then(Value::as_bool) != Some(true) {
                complete = true;
                break;
            }
            let (Some(after), Some(id)) = (after, id) else {
                break;
            };
            let next = (after.to_string(), id.to_string());
            if !seen.insert(next.clone()) {
                break;
            }
            cursor = Some(next);
        }
        let rows = currency
            .map(|code| {
                if code == "USD" {
                    lines::dollar_value(TITLE, sum)
                } else {
                    lines::count_value(TITLE, sum, &code)
                }
            })
            .into_iter()
            .collect();
        Ok(Reading::new(None, rows).with_warning((!complete).then(|| PARTIAL.into())))
    }
}

/// The text at `key`: `None` when it is missing or null, and a decoding error when it holds
/// anything other than a string.
fn optional_text<'a>(object: &'a Value, key: &str) -> Result<Option<&'a str>, SimpleProviderError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        _ => Err(http::decoding(NAME)),
    }
}

/// A cost written as a plain decimal, optionally signed and with an exponent from -128 to 127;
/// `None` for anything else, such as `NaN`, `Infinity` or `2.3.4`.
fn decimal(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    let marker = raw.find(['e', 'E']);
    if let Some(position) = marker {
        let exponent = raw[position + 1..].parse::<i32>().ok()?;
        if !(-128..=127).contains(&exponent) {
            return None;
        }
    }
    let mantissa = raw[..marker.unwrap_or(raw.len())]
        .strip_prefix(['+', '-'])
        .unwrap_or(&raw[..marker.unwrap_or(raw.len())]);
    if mantissa.is_empty()
        || mantissa
            .chars()
            .any(|character| !character.is_ascii_digit() && character != '.')
        || mantissa.matches('.').count() > 1
        || !mantissa.chars().any(|character| character.is_ascii_digit())
    {
        return None;
    }
    value::as_number(&Value::String(raw.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{DateTime, TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const FIRST_PAGE: &str = r#"{"data":[{"cost":"12.00000000","currency":"JPY"},{"cost":"0.50000000","currency":"JPY"}],"has_more":true,"next_after":"time +","next_after_id":"id/1"}"#;
    const LAST_PAGE: &str = r#"{"data":[{"cost":"7.02344000","currency":"JPY"},{"cost":"1.10000000","currency":"JPY"},{"cost":null,"currency":"JPY"},{"cost":"99","currency":"USD"}],"has_more":false}"#;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 0, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn costs_in_the_first_currency_are_summed_across_pages_with_both_cursors_encoded() {
        let http = Scripted::new()
            .on("GET", URL, 200, FIRST_PAGE)
            .on("GET", URL, 200, LAST_PAGE);
        let scope = context_at(&http, json!({"apiKey":"fixture"}), now());
        assert_eq!(
            AiAnd.fetch(&scope.context()).await.unwrap(),
            Reading::new(None, vec![lines::count_value(TITLE, 20.62344, "JPY")])
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            header(&requests[0], "authorization"),
            Some("Bearer fixture")
        );
        assert!(requests[1].url.contains("after=time+%2B&after_id=id%2F1"));
    }

    #[tokio::test]
    async fn the_four_request_cap_keeps_the_pages_read_and_warns_the_total_is_partial() {
        let mut http = Scripted::new();
        for page in 0..4 {
            let body = json!({
                "data": [{"cost": "1.5", "currency": "USD"}],
                "has_more": true,
                "next_after": page.to_string(),
                "next_after_id": "row"
            })
            .to_string();
            http = http.on("GET", URL, 200, &body);
        }
        let scope = context_at(&http, json!({"apiKey":"fixture"}), now());
        let reading = AiAnd.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines, vec![lines::dollar_value(TITLE, 6.0)]);
        assert_eq!(reading.warning.as_deref(), Some(PARTIAL));
        assert_eq!(http.requests().len(), 4);
    }

    #[tokio::test]
    async fn refused_keys_rate_limits_and_malformed_pages_keep_their_category() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{", ErrorCategory::Decoding),
            (200, r#"{"data":[{"cost":3}]}"#, ErrorCategory::Decoding),
            (200, "{}", ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"fixture"}), now());
            assert_eq!(
                AiAnd.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }

    #[test]
    fn costs_are_read_only_as_plain_decimals_with_an_exponent_up_to_127() {
        for text in ["NaN", "Infinity", "1e128", "++2", ".", "2.3.4"] {
            assert_eq!(decimal(text), None);
        }
        assert_eq!(decimal("1.25e2"), Some(125.0));
    }
}
