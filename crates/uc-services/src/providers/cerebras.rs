//! Cerebras: the request and token rate limits of a Cerebras API key.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `CEREBRAS_API_KEY` environment variable or from a key saved in Accounts. A refresh sends one
//! request, `GET https://api.cerebras.ai/v1/models` with the key as a bearer token, and never an
//! inference or completion request. Cerebras documents request and token limits per minute, hour
//! and day (https://inference-docs.cerebras.ai/support/rate-limits.md) but not the format of the
//! headers that report them or of their resets, so a meter is shown only for a bucket whose
//! `x-ratelimit-limit-<bucket>` and `x-ratelimit-remaining-<bucket>` headers the server actually
//! returned, with a positive limit and a remaining count between zero and that limit. Reset times
//! stay unknown, and an answer without any such pair is reported as unavailable rather than as
//! zero.

use async_trait::async_trait;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines};

pub(crate) struct Cerebras;

const MODELS_URL: &str = "https://api.cerebras.ai/v1/models";

/// Each rate-limit bucket: its name in the `x-ratelimit-*` headers, its meter title, its unit and
/// the length of its window.
const BUCKETS: [(&str, &str, &str, i64); 6] = [
    ("requests-minute", "Requests Per Minute", "requests", 60000),
    (
        "requests-hour",
        "Requests Per Hour",
        "requests",
        lines::HOUR_MS,
    ),
    (
        "requests-day",
        "Requests Per Day",
        "requests",
        lines::DAY_MS,
    ),
    ("tokens-minute", "Tokens Per Minute", "tokens", 60000),
    ("tokens-hour", "Tokens Per Hour", "tokens", lines::HOUR_MS),
    ("tokens-day", "Tokens Per Day", "tokens", lines::DAY_MS),
];

/// The widget id suffix of a bucket: `requests-minute` becomes `requestsMinute`.
fn descriptor_suffix(bucket: &str) -> String {
    let (noun, period) = bucket.split_once('-').unwrap_or((bucket, ""));
    let mut chars = period.chars();
    format!(
        "{noun}{}{}",
        chars
            .next()
            .map(|first| first.to_ascii_uppercase())
            .unwrap_or_default(),
        chars.as_str()
    )
}

#[async_trait]
impl Service for Cerebras {
    fn id(&self) -> &'static str {
        "cerebras"
    }

    fn name(&self) -> &'static str {
        "Cerebras"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["CEREBRAS_API_KEY"],
            url: "https://cloud.cerebras.ai/platform",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        BUCKETS
            .iter()
            .map(|(bucket, title, unit, period)| {
                WidgetDescriptor::bounded_count(
                    format!("{}.{}", provider.id, descriptor_suffix(bucket)),
                    provider,
                    title,
                    None,
                    0.0,
                    unit,
                    Some(*period),
                )
                .exporting_progress(&descriptor_suffix(bucket), unit)
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Cerebras API key is missing."))?;
        let response = http::send(
            context.http,
            HttpRequest::get(MODELS_URL).bearer(key),
            "Cerebras",
        )
        .await?;
        if !response.is_success() {
            return Err(http::status_error(&response, "Cerebras"));
        }
        let mut rows = Vec::new();
        for (s, title, unit, period) in BUCKETS {
            let read = |part: &str| {
                response
                    .header(&format!("x-ratelimit-{part}-{s}"))
                    .and_then(|text| text.trim().parse::<f64>().ok())
                    .filter(|number| number.is_finite())
            };
            if let (Some(limit), Some(left)) = (read("limit"), read("remaining"))
                && limit > 0.0
                && left >= 0.0
                && left <= limit
            {
                rows.push(lines::count(
                    title,
                    limit - left,
                    limit,
                    unit,
                    None,
                    Some(period),
                ));
            }
        }
        if rows.is_empty() {
            return Err(http::not_available(
                "Cerebras did not provide readable rate-limit headers on its model list. No inference request was sent.",
            ));
        }
        Ok(Reading::new(None, rows))
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
    async fn only_the_buckets_the_server_reported_become_meters() {
        let http = Scripted::new().on_with_headers(
            "GET",
            MODELS_URL,
            200,
            &[
                ("x-ratelimit-limit-requests-day", "100"),
                ("x-ratelimit-remaining-requests-day", "25"),
                ("x-ratelimit-limit-tokens-minute", "1000"),
                ("x-ratelimit-remaining-tokens-minute", "750"),
                ("x-ratelimit-reset-tokens-minute", "30"),
            ],
            r#"{"object":"list","data":[]}"#,
        );
        let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
        let reading = Cerebras.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![
                lines::count(
                    "Requests Per Day",
                    75.0,
                    100.0,
                    "requests",
                    None,
                    Some(lines::DAY_MS)
                ),
                lines::count(
                    "Tokens Per Minute",
                    250.0,
                    1000.0,
                    "tokens",
                    None,
                    Some(60000)
                )
            ]
        );
        assert_eq!(http.requests().len(), 1);
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some("Bearer test")
        );
        assert!(http.requests()[0].body.is_none());
    }

    #[test]
    fn descriptor_ids_and_exports_use_camel_case_bucket_names() {
        let provider = Provider::new("cerebras", "Cerebras");
        let descriptors = Cerebras.descriptors(&provider);
        assert_eq!(descriptors[0].id, "cerebras.requestsMinute");
        assert_eq!(descriptors[0].metric_label, "Requests Per Minute");
        assert_eq!(descriptors[0].limit_resources[0].key, "requestsMinute");
        assert_eq!(descriptors[5].id, "cerebras.tokensDay");
    }

    #[tokio::test]
    async fn missing_or_unreadable_headers_are_unavailable_rather_than_zero() {
        for headers in [
            vec![],
            vec![("x-ratelimit-limit-requests-day", "100")],
            vec![
                ("x-ratelimit-limit-requests-day", "NaN"),
                ("x-ratelimit-remaining-requests-day", "20"),
            ],
        ] {
            let http = Scripted::new().on_with_headers("GET", MODELS_URL, 200, &headers, "{}");
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            assert_eq!(
                Cerebras.fetch(&scope.context()).await.unwrap_err().category,
                ErrorCategory::NotAvailable
            );
        }
    }

    #[tokio::test]
    async fn refused_rate_limited_and_server_errors_keep_their_categories() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (503, ErrorCategory::Http5xx),
        ] {
            let http = Scripted::new().on("GET", MODELS_URL, status, "{}");
            let scope = context_at(&http, json!({"apiKey":"test"}), Utc::now());
            assert_eq!(
                Cerebras.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }
}
