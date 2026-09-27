//! Groq: a GroqCloud API key, saved in Quota Control or set in `GROQ_API_KEY`, read against the
//! rate limits GroqCloud attaches to every API response. Nothing is read from disk on Windows,
//! macOS or Linux. Each refresh sends one `GET https://api.groq.com/openai/v1/models` (a model
//! list: it runs no model, so it spends no tokens) and reads only its `x-ratelimit-*` headers:
//! requests per day and tokens per minute, each with the time its bucket takes to refill.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use uc_core::{
    HttpRequest, HttpResponse, MetricLine, Provider, ProviderLink, SimpleProviderError,
    WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines};

pub(crate) struct Groq;

const NAME: &str = "Groq";
const MODELS: &str = "https://api.groq.com/openai/v1/models";
const MINUTE_MS: i64 = 60_000;
/// A 401 (the key is not valid) or 403 (the key lacks permission) from GroqCloud: an API key has
/// no login to renew, so the card asks for a working key like the other key-only services.
const KEY_REFUSED: &str = "Groq refused this API key. Check it or create a new one.";

/// One rate-limit bucket GroqCloud reports in its response headers.
struct Bucket {
    suffix: &'static str,
    title: &'static str,
    /// The header family (`x-ratelimit-{limit,remaining,reset}-<noun>`), also the counted unit.
    noun: &'static str,
    period_ms: i64,
    /// The limit Groq's documentation shows, only a placeholder for the widget template.
    sample_limit: f64,
}

/// GroqCloud's request headers always count requests per day and its token headers tokens per
/// minute (console.groq.com/docs/rate-limits).
const BUCKETS: [Bucket; 2] = [
    Bucket {
        suffix: "requests",
        title: "Requests",
        noun: "requests",
        period_ms: lines::DAY_MS,
        sample_limit: 14_400.0,
    },
    Bucket {
        suffix: "tokensPerMinute",
        title: "Tokens per Minute",
        noun: "tokens",
        period_ms: MINUTE_MS,
        sample_limit: 18_000.0,
    },
];

#[async_trait]
impl Service for Groq {
    fn id(&self) -> &'static str {
        "groq"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Status", "https://groqstatus.com"),
            ProviderLink::new("Limits", "https://console.groq.com/settings/limits"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["GROQ_API_KEY"],
            url: "https://console.groq.com/keys",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        BUCKETS
            .iter()
            .map(|bucket| {
                WidgetDescriptor::bounded_count(
                    format!("{}.{}", provider.id, bucket.suffix),
                    provider,
                    bucket.title,
                    None,
                    bucket.sample_limit,
                    bucket.noun,
                    Some(bucket.period_ms),
                )
                .exporting_progress(bucket.suffix, bucket.noun)
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context.secret.key().ok_or_else(|| {
            http::invalid("The Groq API key is missing. Add it again in Accounts.")
        })?;
        let response = http::send(
            context.http,
            HttpRequest::get(MODELS)
                .bearer(key)
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        if matches!(response.status, 401 | 403) {
            return Err(http::invalid(KEY_REFUSED));
        }
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        let meters = meters(&response, context.now);
        let warning = meters
            .is_empty()
            .then(|| "Groq reported no rate limits for this key.".to_string());
        Ok(Reading::new(None, meters).with_warning(warning))
    }
}

/// One meter per bucket whose limit and remaining headers are both readable, so a missing header
/// never shows as an empty bucket. The reset header is how long the bucket takes to refill.
fn meters(response: &HttpResponse, now: DateTime<Utc>) -> Vec<MetricLine> {
    BUCKETS
        .iter()
        .filter_map(|bucket| {
            let limit = rate_header(response, "limit", bucket.noun)
                .and_then(number)
                .filter(|limit| *limit > 0.0)?;
            let remaining = rate_header(response, "remaining", bucket.noun).and_then(number)?;
            let resets_at = rate_header(response, "reset", bucket.noun)
                .and_then(duration_ms)
                .and_then(Duration::try_milliseconds)
                .and_then(|wait| now.checked_add_signed(wait));
            Some(lines::count(
                bucket.title,
                (limit - remaining).clamp(0.0, limit),
                limit,
                bucket.noun,
                resets_at,
                Some(bucket.period_ms),
            ))
        })
        .collect()
}

fn rate_header<'a>(response: &'a HttpResponse, part: &str, noun: &str) -> Option<&'a str> {
    response.header(&format!("x-ratelimit-{part}-{noun}"))
}

/// A header's decimal number.
fn number(text: &str) -> Option<f64> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
}

/// A Go duration such as `2m59.56s`, `7.66s` or `850ms` in whole milliseconds; a bare number is
/// seconds. Negative or malformed text gives `None`.
fn duration_ms(text: &str) -> Option<i64> {
    let text = text.trim();
    if let Ok(seconds) = text.parse::<f64>() {
        return (seconds.is_finite() && seconds >= 0.0).then(|| (seconds * 1000.0).round() as i64);
    }
    let mut rest = text;
    let mut total = 0.0;
    while !rest.is_empty() {
        let digits = rest
            .find(|character: char| !character.is_ascii_digit() && character != '.')
            .unwrap_or(rest.len());
        let amount: f64 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let unit = rest
            .find(|character: char| character.is_ascii_digit() || character == '.')
            .unwrap_or(rest.len());
        let scale = match &rest[..unit] {
            "h" => 3_600_000.0,
            "m" => 60_000.0,
            "s" => 1_000.0,
            "ms" => 1.0,
            "us" | "\u{b5}s" | "\u{3bc}s" => 0.001,
            "ns" => 0.000_001,
            _ => return None,
        };
        total += amount * scale;
        rest = &rest[unit..];
    }
    (!text.is_empty() && total.is_finite()).then(|| total.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Roots;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use uc_core::{ErrorCategory, MetricKind, ProgressFormat, ProgressLine};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn key() -> Value {
        json!({"apiKey": "gsk_test"})
    }

    /// The headers from Groq's rate-limit documentation.
    const DOCUMENTED: [(&str, &str); 6] = [
        ("x-ratelimit-limit-requests", "14400"),
        ("x-ratelimit-limit-tokens", "18000"),
        ("x-ratelimit-remaining-requests", "14370"),
        ("x-ratelimit-remaining-tokens", "17997"),
        ("x-ratelimit-reset-requests", "2m59.56s"),
        ("x-ratelimit-reset-tokens", "7.66s"),
    ];

    const MODEL_LIST: &str = r#"{"object":"list","data":[{"id":"llama-3.3-70b-versatile","object":"model","created":1733447754,"owned_by":"Meta","active":true,"context_window":131072},{"id":"openai/gpt-oss-120b","object":"model","created":1754408224,"owned_by":"OpenAI","active":true,"context_window":131072}]}"#;

    fn meter(
        label: &str,
        used: f64,
        limit: f64,
        suffix: &str,
        resets_at: Option<DateTime<Utc>>,
        period_ms: i64,
    ) -> MetricLine {
        MetricLine::Progress(ProgressLine {
            label: label.into(),
            used,
            limit,
            format: ProgressFormat::Count {
                suffix: suffix.into(),
            },
            resets_at,
            period_duration_ms: Some(period_ms),
            color_hex: None,
        })
    }

    fn response(headers: &[(&str, &str)]) -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: headers
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect::<HashMap<_, _>>(),
            body: Vec::new(),
        }
    }

    #[tokio::test]
    async fn reads_requests_per_day_and_tokens_per_minute_from_the_headers() {
        let http = Scripted::new().on_with_headers("GET", MODELS, 200, &DOCUMENTED, MODEL_LIST);
        let scope = context_at(&http, key(), now());
        let reading = Groq.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, None);
        assert_eq!(reading.warning, None);
        assert_eq!(
            reading.lines,
            vec![
                meter(
                    "Requests",
                    30.0,
                    14_400.0,
                    "requests",
                    Some(
                        Utc.with_ymd_and_hms(2026, 9, 27, 10, 2, 59).unwrap()
                            + Duration::milliseconds(560)
                    ),
                    86_400_000,
                ),
                meter(
                    "Tokens per Minute",
                    3.0,
                    18_000.0,
                    "tokens",
                    Some(
                        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 7).unwrap()
                            + Duration::milliseconds(660)
                    ),
                    60_000,
                ),
            ]
        );
    }

    #[tokio::test]
    async fn sends_one_model_list_request_with_the_key_as_bearer() {
        let http = Scripted::new().on_with_headers("GET", MODELS, 200, &DOCUMENTED, MODEL_LIST);
        let scope = context_at(&http, json!({"apiKey": "  gsk_test \n"}), now());
        Groq.fetch(&scope.context()).await.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].url, "https://api.groq.com/openai/v1/models");
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some("Bearer gsk_test")
        );
        assert_eq!(header(&requests[0], "Accept"), Some("application/json"));
        assert_eq!(requests[0].body, None);
        assert_eq!(requests[0].timeout, HttpRequest::DEFAULT_TIMEOUT);
    }

    #[tokio::test]
    async fn a_rejected_key_is_reported_without_echoing_it() {
        for (status, body) in [
            (
                401,
                r#"{"error":{"message":"Invalid API Key","type":"invalid_request_error","code":"invalid_api_key"}}"#,
            ),
            (
                403,
                r#"{"error":{"message":"Forbidden","type":"invalid_request_error"}}"#,
            ),
        ] {
            let http = Scripted::new().on("GET", MODELS, status, body);
            let scope = context_at(&http, key(), now());
            let error = Groq.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthInvalid, "{status}");
            assert_eq!(
                error.message, "Groq refused this API key. Check it or create a new one.",
                "{status}"
            );
            assert!(!error.message.contains("gsk_test"), "{status}");
        }
    }

    #[tokio::test]
    async fn too_many_requests_is_reported_as_rate_limited() {
        let http = Scripted::new().on_with_headers(
            "GET",
            MODELS,
            429,
            &[("retry-after", "2")],
            r#"{"error":{"message":"Rate limit reached for requests","type":"requests","code":"rate_limit_exceeded"}}"#,
        );
        let scope = context_at(&http, key(), now());
        let error = Groq.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
        assert_eq!(
            error.message,
            "Groq is rate limiting usage requests. Waiting before retrying."
        );
    }

    #[tokio::test]
    async fn a_server_failure_is_reported_by_status() {
        let http = Scripted::new().on("GET", MODELS, 503, "upstream unavailable");
        let scope = context_at(&http, key(), now());
        let error = Groq.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Http5xx);
        assert_eq!(error.message, "Groq answered with HTTP 503.");
    }

    #[tokio::test]
    async fn a_key_without_rate_limit_headers_refreshes_with_a_notice() {
        let http = Scripted::new().on("GET", MODELS, 200, MODEL_LIST);
        let scope = context_at(&http, key(), now());
        let reading = Groq.fetch(&scope.context()).await.unwrap();
        assert!(reading.lines.is_empty());
        assert_eq!(
            reading.warning.as_deref(),
            Some("Groq reported no rate limits for this key.")
        );
    }

    #[tokio::test]
    async fn a_secret_without_a_key_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({"label": "Groq"}), now());
        let error = Groq.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert!(http.requests().is_empty());
    }

    #[test]
    fn a_bucket_needs_both_its_limit_and_what_remains() {
        let lines = meters(
            &response(&[
                ("x-ratelimit-limit-requests", "1000"),
                ("x-ratelimit-remaining-requests", "0"),
                ("x-ratelimit-reset-requests", "soon"),
                ("x-ratelimit-limit-tokens", "6000"),
                ("x-ratelimit-reset-tokens", "1s"),
            ]),
            now(),
        );
        assert_eq!(
            lines,
            vec![meter(
                "Requests", 1000.0, 1000.0, "requests", None, 86_400_000
            )]
        );
        let unreadable = meters(
            &response(&[
                ("x-ratelimit-limit-requests", "0"),
                ("x-ratelimit-remaining-requests", "0"),
                ("x-ratelimit-limit-tokens", "6000"),
                ("x-ratelimit-remaining-tokens", "lots"),
            ]),
            now(),
        );
        assert!(unreadable.is_empty());
    }

    #[test]
    fn used_stays_between_zero_and_the_limit() {
        let lines = meters(
            &response(&[
                ("x-ratelimit-limit-requests", "1000"),
                ("x-ratelimit-remaining-requests", "1200"),
                ("x-ratelimit-limit-tokens", "6000"),
                ("x-ratelimit-remaining-tokens", "-250"),
                ("x-ratelimit-reset-tokens", "12ms"),
            ]),
            now(),
        );
        assert_eq!(
            lines,
            vec![
                meter("Requests", 0.0, 1000.0, "requests", None, 86_400_000),
                meter(
                    "Tokens per Minute",
                    6000.0,
                    6000.0,
                    "tokens",
                    Some(now() + Duration::milliseconds(12)),
                    60_000
                ),
            ]
        );
    }

    #[test]
    fn reset_headers_are_go_durations() {
        for (text, expected) in [
            ("2m59.56s", Some(179_560)),
            ("7.66s", Some(7_660)),
            ("1h2m3s", Some(3_723_000)),
            ("24h0m0s", Some(86_400_000)),
            ("850ms", Some(850)),
            ("1.5\u{b5}s", Some(0)),
            ("999\u{3bc}s", Some(1)),
            ("0s", Some(0)),
            (" 12 ", Some(12_000)),
            ("0.25", Some(250)),
            ("", None),
            ("soon", None),
            ("-1s", None),
            ("-5", None),
            ("5x", None),
            ("ms", None),
            ("1.2.3s", None),
            ("7.66 s", None),
            ("inf", None),
        ] {
            assert_eq!(duration_ms(text), expected, "{text:?}");
        }
    }

    #[test]
    fn descriptors_match_the_metric_lines() {
        let provider = Provider::new("groq", "Groq");
        let descriptors = Groq.descriptors(&provider);
        let summary: Vec<_> = descriptors
            .iter()
            .map(|descriptor| {
                (
                    descriptor.id.as_str(),
                    descriptor.metric_label.as_str(),
                    descriptor.template.kind,
                    descriptor.template.count_suffix.as_deref(),
                    descriptor.template.period_duration_ms,
                    descriptor.limit_resources[0].key.as_str(),
                    descriptor.limit_resources[0].unit.as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "groq.requests",
                    "Requests",
                    MetricKind::Count,
                    Some("requests"),
                    Some(86_400_000),
                    "requests",
                    "requests",
                ),
                (
                    "groq.tokensPerMinute",
                    "Tokens per Minute",
                    MetricKind::Count,
                    Some("tokens"),
                    Some(60_000),
                    "tokensPerMinute",
                    "tokens",
                ),
            ]
        );
    }

    #[test]
    fn connects_with_a_console_key_or_the_environment() {
        let connection = Groq.connection();
        assert_eq!(connection.login_from, None);
        let help = connection.api_key.unwrap();
        assert_eq!(help.env.to_vec(), vec!["GROQ_API_KEY"]);
        assert_eq!(help.url, "https://console.groq.com/keys");
        assert!(help.fields.is_empty());
        let dir = tempfile::tempdir().unwrap();
        assert!(Groq.discover(&Roots::under(dir.path())).is_empty());
        let links: Vec<_> = Groq
            .links()
            .into_iter()
            .map(|link| (link.label, link.url))
            .collect();
        assert_eq!(
            links,
            vec![
                ("Status".to_string(), "https://groqstatus.com".to_string()),
                (
                    "Limits".to_string(),
                    "https://console.groq.com/settings/limits".to_string()
                ),
            ]
        );
    }
}
