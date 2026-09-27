//! OpenAI: what an organization spent on the OpenAI API this month and today, read with an
//! organization Admin API key.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key is an Admin API key (`sk-admin-…`,
//! created by an organization owner) that the user saves in Accounts or sets in `OPENAI_ADMIN_KEY`.
//! `OPENAI_API_KEY` is left alone on purpose: project and service account keys cannot read
//! organization costs, so a card made from one could only show an error.
//!
//! A refresh calls `GET https://api.openai.com/v1/organization/costs` with the key as a bearer
//! token, for daily buckets (`bucket_width=1d`) from midnight UTC on the first of the current month
//! to midnight UTC tomorrow, and follows `next_page` while `has_more` is set (31 buckets a page, so
//! one page holds the whole month). The US dollar amounts of the month's buckets add up to Spend
//! This Month, and today's UTC bucket is Today. OpenAI reports spend only: the endpoint has no
//! balance, cap or plan.

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Utc};
use serde_json::Value;
use uc_core::{
    HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind, Provider, ProviderLink,
    SimpleProviderError, WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct OpenAi;

const NAME: &str = "OpenAI";
const COSTS_URL: &str = "https://api.openai.com/v1/organization/costs";
const ADMIN_KEYS_URL: &str = "https://platform.openai.com/settings/organization/admin-keys";
/// How every OpenAI Admin API key starts; only the wording of a refusal depends on it.
const ADMIN_KEY_PREFIX: &str = "sk-admin-";

const THIS_MONTH: &str = "Spend This Month";
const TODAY: &str = "Today";

const DAY_SECONDS: i64 = 86_400;
/// Buckets asked for per page: a month has at most 31 days (the endpoint allows up to 180).
const BUCKETS_PER_PAGE: u32 = 31;
/// Pages read in one refresh at most; a month normally fits in the first.
const MAX_PAGES: usize = 4;

const NO_KEY: &str = "The OpenAI Admin API key is missing. Add it again in Accounts.";
const NOT_ADMIN: &str = "OpenAI usage needs an Admin API key. Project and service account keys cannot read organization costs.";
const ADMIN_REFUSED: &str = "OpenAI refused this Admin API key. Check it or create a new one.";

#[async_trait]
impl Service for OpenAi {
    fn id(&self) -> &'static str {
        "openai"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Status", "https://status.openai.com"),
            ProviderLink::new("Usage", "https://platform.openai.com/usage"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["OPENAI_ADMIN_KEY"],
            url: ADMIN_KEYS_URL,
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [("month", THIS_MONTH), ("today", TODAY)]
            .into_iter()
            .map(|(suffix, title)| {
                WidgetDescriptor::values(
                    format!("{}.{suffix}", provider.id),
                    provider,
                    title,
                    None,
                    Some(MetricKind::Dollars),
                    None,
                    true,
                    None,
                    false,
                )
                .exporting_limit(
                    suffix,
                    LimitResourceKind::Consumption,
                    "usd",
                    LimitResourceSource::Value {
                        kind: MetricKind::Dollars,
                        label: None,
                    },
                    false,
                )
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context.secret.key().ok_or_else(|| http::invalid(NO_KEY))?;
        let (today, month) = day_and_month_start(context.now);
        let mut spend = Spend::default();
        let mut cursors: Vec<String> = Vec::new();
        for _ in 0..MAX_PAGES {
            let cursor = cursors.last().map(String::as_str);
            let page = costs_page(context, key, month, today, cursor).await?;
            spend
                .add(&page, month, today)
                .ok_or_else(|| http::decoding(NAME))?;
            match next_page(&page)? {
                None => return Ok(spend.reading()),
                // A cursor seen before would read the same buckets again and count them twice.
                Some(next) if cursors.contains(&next) => return Err(http::decoding(NAME)),
                Some(next) => cursors.push(next),
            }
        }
        Err(http::decoding(NAME))
    }
}

/// One page of the month's daily cost buckets. A refused key is told apart from a key that is not
/// an Admin key, the only kind the endpoint accepts.
async fn costs_page(
    context: &FetchContext<'_>,
    key: &str,
    month: i64,
    today: i64,
    cursor: Option<&str>,
) -> Result<Value, SimpleProviderError> {
    let response = http::send(
        context.http,
        HttpRequest::get(costs_url(month, today + DAY_SECONDS, cursor))
            .bearer(key)
            .header("Accept", "application/json"),
        NAME,
    )
    .await?;
    if matches!(response.status, 401 | 403) {
        return Err(http::invalid(if key.starts_with(ADMIN_KEY_PREFIX) {
            ADMIN_REFUSED
        } else {
            NOT_ADMIN
        }));
    }
    if !response.is_success() {
        return Err(http::status_error(&response, NAME));
    }
    http::parse(&response, NAME)
}

/// Daily buckets from `start` (inclusive) to `end` (exclusive), both Unix seconds.
fn costs_url(start: i64, end: i64, cursor: Option<&str>) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query
        .append_pair("start_time", &start.to_string())
        .append_pair("end_time", &end.to_string())
        .append_pair("bucket_width", "1d")
        .append_pair("limit", &BUCKETS_PER_PAGE.to_string());
    if let Some(cursor) = cursor {
        query.append_pair("page", cursor);
    }
    format!("{COSTS_URL}?{}", query.finish())
}

/// The cursor of the page after `page`, `None` on the last one. A page that does not say whether
/// more follow, or says so but names no cursor, cannot be read as the whole month.
fn next_page(page: &Value) -> Result<Option<String>, SimpleProviderError> {
    match value::flag(page, "/has_more") {
        Some(false) => Ok(None),
        Some(true) => value::text(page, "/next_page")
            .map(|cursor| Some(cursor.to_string()))
            .ok_or_else(|| http::decoding(NAME)),
        None => Err(http::decoding(NAME)),
    }
}

/// Midnight UTC today and on the first of this month, in Unix seconds (a UTC day is always
/// 86,400 of them).
fn day_and_month_start(now: DateTime<Utc>) -> (i64, i64) {
    let today = now.timestamp().div_euclid(DAY_SECONDS) * DAY_SECONDS;
    (today, today - i64::from(now.day0()) * DAY_SECONDS)
}

/// The US dollars summed from the cost buckets read so far.
#[derive(Default)]
struct Spend {
    month: f64,
    today: f64,
}

impl Spend {
    /// Adds one page: each bucket from the first of the month counts toward the month, and the one
    /// starting at today's midnight toward today too. `None` when the page is not the documented
    /// shape, so a changed answer never reads as a smaller spend.
    fn add(&mut self, page: &Value, month: i64, today: i64) -> Option<()> {
        for bucket in page.get("data")?.as_array()? {
            let start = value::number(bucket, "/start_time")? as i64;
            let mut total = 0.0;
            for result in bucket.get("results")?.as_array()? {
                total += amount(result)?;
            }
            if start >= month {
                self.month += total;
            }
            if (today..today + DAY_SECONDS).contains(&start) {
                self.today += total;
            }
        }
        Some(())
    }

    /// Negative amounts net against the rest, but a total never shows below zero.
    fn reading(&self) -> Reading {
        Reading::new(
            None,
            vec![
                lines::dollar_value(THIS_MONTH, self.month.max(0.0)),
                lines::dollar_value(TODAY, self.today.max(0.0)),
            ],
        )
    }
}

/// The US dollar amount of one cost result: zero when it carries none, `None` when it is not a
/// result object or its amount is not a number in US dollars.
fn amount(result: &Value) -> Option<f64> {
    let Some(amount) = result
        .as_object()?
        .get("amount")
        .filter(|amount| !amount.is_null())
    else {
        return Some(0.0);
    };
    let amount = amount.as_object()?;
    let currency = amount
        .get("currency")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !currency.is_empty() && !currency.eq_ignore_ascii_case("usd") {
        return None;
    }
    match amount.get("value") {
        None | Some(Value::Null) => Some(0.0),
        Some(Value::String(text)) if text.trim().is_empty() => Some(0.0),
        Some(raw) => value::as_number(raw),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Roots;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;
    use serde_json::json;
    use uc_core::{ErrorCategory, MetricLine};

    const ADMIN_KEY: &str = "sk-admin-test-0123456789abcdef";
    const PROJECT_KEY: &str = "sk-proj-test-0123456789abcdef";

    const DAY: i64 = 86_400;
    /// Midnight UTC on 1 September 2026.
    const SEP_1: i64 = 1_788_220_800;
    /// Midnight UTC on 27 September 2026.
    const SEP_27: i64 = 1_790_467_200;

    /// Sunday 27 September 2026, 10:00 UTC.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    /// The first page asked for on 27 September 2026: 1 September up to 28 September.
    fn first_page() -> String {
        "https://api.openai.com/v1/organization/costs?start_time=1788220800&end_time=1790553600&bucket_width=1d&limit=31".into()
    }

    /// One cost result the way the endpoint answers it without grouping.
    fn cost(value: Value) -> Value {
        json!({
            "object": "organization.costs.result",
            "amount": {"value": value, "currency": "usd"},
            "line_item": null,
            "project_id": null,
            "api_key_id": null,
            "quantity": null
        })
    }

    fn bucket(start: i64, results: Vec<Value>) -> Value {
        json!({
            "object": "bucket",
            "start_time": start,
            "end_time": start + DAY,
            "results": results
        })
    }

    fn page(buckets: Vec<Value>, next: Option<&str>) -> String {
        json!({
            "object": "page",
            "data": buckets,
            "has_more": next.is_some(),
            "next_page": next
        })
        .to_string()
    }

    fn spend(month: f64, today: f64) -> Vec<MetricLine> {
        vec![
            lines::dollar_value("Spend This Month", month),
            lines::dollar_value("Today", today),
        ]
    }

    async fn answer(
        key: &str,
        status: u16,
        body: &str,
    ) -> (Scripted, Result<Reading, SimpleProviderError>) {
        let http = Scripted::new().on("GET", COSTS_URL, status, body);
        let scope = context_at(&http, json!({ "apiKey": key }), now());
        let result = OpenAi.fetch(&scope.context()).await;
        (http, result)
    }

    async fn read(body: &str) -> Reading {
        answer(ADMIN_KEY, 200, body).await.1.unwrap()
    }

    async fn fail(key: &str, status: u16, body: &str) -> SimpleProviderError {
        answer(key, status, body).await.1.unwrap_err()
    }

    #[tokio::test]
    async fn adds_up_this_months_dollar_costs_and_picks_out_today() {
        let reading = read(&page(
            vec![
                bucket(SEP_1, vec![cost(json!(12.5))]),
                bucket(SEP_1 + DAY, vec![]),
                bucket(
                    SEP_27 - DAY,
                    vec![
                        cost(json!("2.25")),
                        json!({"object": "organization.costs.result", "amount": null}),
                    ],
                ),
                bucket(SEP_27, vec![cost(json!(1.5)), cost(json!(0.25))]),
            ],
            None,
        ))
        .await;
        assert_eq!(reading, Reading::new(None, spend(16.5, 1.75)));
        let provider = Provider::new("openai@abc", "OpenAI");
        let labels: Vec<String> = OpenAi
            .descriptors(&provider)
            .into_iter()
            .map(|descriptor| descriptor.metric_label)
            .collect();
        for line in &reading.lines {
            assert!(labels.iter().any(|label| label == line.label()), "{line:?}");
        }
    }

    #[tokio::test]
    async fn asks_for_this_months_daily_buckets_with_the_admin_key() {
        let (http, result) = answer(ADMIN_KEY, 200, &page(Vec::new(), None)).await;
        result.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, "GET");
        assert_eq!(request.url, first_page());
        assert_eq!(
            header(request, "Authorization"),
            Some("Bearer sk-admin-test-0123456789abcdef")
        );
        assert_eq!(header(request, "Accept"), Some("application/json"));
        assert_eq!(request.body, None);
        assert_eq!(request.timeout, HttpRequest::DEFAULT_TIMEOUT);
    }

    #[tokio::test]
    async fn a_month_without_costs_shows_real_zeros() {
        let reading = read(&page(
            vec![bucket(SEP_1, Vec::new()), bucket(SEP_27, Vec::new())],
            None,
        ))
        .await;
        assert_eq!(reading.lines, spend(0.0, 0.0));
        assert_eq!(read(&page(Vec::new(), None)).await.lines, spend(0.0, 0.0));
    }

    #[tokio::test]
    async fn follows_next_page_until_the_last_one() {
        let second_page = format!("{}&page=page_AAAAAGnVd3o%3D", first_page());
        let http = Scripted::new()
            .on(
                "GET",
                &first_page(),
                200,
                &page(
                    vec![bucket(SEP_1, vec![cost(json!(4.0))])],
                    Some("page_AAAAAGnVd3o="),
                ),
            )
            .on(
                "GET",
                &second_page,
                200,
                &page(vec![bucket(SEP_27, vec![cost(json!(0.5))])], None),
            );
        let scope = context_at(&http, json!({ "apiKey": ADMIN_KEY }), now());
        let reading = OpenAi.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines, spend(4.5, 0.5));
        let urls: Vec<String> = http
            .requests()
            .into_iter()
            .map(|request| request.url)
            .collect();
        assert_eq!(urls, [first_page(), second_page]);
    }

    #[tokio::test]
    async fn gives_up_after_four_pages_that_all_say_more_follow() {
        let mut http = Scripted::new();
        for number in 1..=5 {
            http = http.on(
                "GET",
                COSTS_URL,
                200,
                &page(Vec::new(), Some(&format!("page_{number}"))),
            );
        }
        let scope = context_at(&http, json!({ "apiKey": ADMIN_KEY }), now());
        let error = OpenAi.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Decoding);
        assert_eq!(http.requests().len(), 4);
    }

    #[tokio::test]
    async fn a_new_month_counts_from_midnight_utc_on_the_first() {
        let oct_1 = SEP_1 + 30 * DAY;
        let http = Scripted::new().on(
            "GET",
            COSTS_URL,
            200,
            &page(
                vec![
                    bucket(oct_1 - DAY, vec![cost(json!(9.0))]),
                    bucket(oct_1, vec![cost(json!(0.75))]),
                ],
                None,
            ),
        );
        let first_minutes = Utc.with_ymd_and_hms(2026, 10, 1, 0, 5, 0).unwrap();
        let scope = context_at(&http, json!({ "apiKey": ADMIN_KEY }), first_minutes);
        let reading = OpenAi.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines, spend(0.75, 0.75));
        assert_eq!(
            http.requests()[0].url,
            "https://api.openai.com/v1/organization/costs?start_time=1790812800&end_time=1790899200&bucket_width=1d&limit=31"
        );
    }

    #[tokio::test]
    async fn negative_amounts_net_out_but_a_total_never_shows_below_zero() {
        let reading = read(&page(
            vec![
                bucket(SEP_1, vec![cost(json!(5.0)), cost(json!(-1.5))]),
                bucket(SEP_27, vec![cost(json!(-2.0))]),
            ],
            None,
        ))
        .await;
        assert_eq!(reading.lines, spend(1.5, 0.0));
    }

    #[tokio::test]
    async fn a_project_key_is_told_that_usage_needs_an_admin_key() {
        for (status, body) in [
            (
                401,
                r#"{"error":{"message":"Incorrect API key provided: sk-proj-****cdef.","type":"invalid_request_error","param":null,"code":"invalid_api_key"}}"#,
            ),
            (
                403,
                r#"{"error":{"message":"You have insufficient permissions for this operation.","type":"invalid_request_error","param":null,"code":null}}"#,
            ),
        ] {
            let error = fail(PROJECT_KEY, status, body).await;
            assert_eq!(error.category, ErrorCategory::AuthInvalid, "{status}");
            assert_eq!(
                error.message,
                "OpenAI usage needs an Admin API key. Project and service account keys cannot read organization costs."
            );
            assert!(!error.message.contains(PROJECT_KEY));
        }
    }

    #[tokio::test]
    async fn a_refused_admin_key_is_reported_without_revealing_it() {
        let error = fail(
            ADMIN_KEY,
            401,
            r#"{"error":{"message":"Incorrect API key provided: sk-admin-****cdef.","type":"invalid_request_error","param":null,"code":"invalid_api_key"}}"#,
        )
        .await;
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(
            error.message,
            "OpenAI refused this Admin API key. Check it or create a new one."
        );
        assert!(!error.message.contains(ADMIN_KEY));
    }

    #[tokio::test]
    async fn rate_limits_and_server_errors_keep_their_categories() {
        let limited = fail(
            ADMIN_KEY,
            429,
            r#"{"error":{"message":"Rate limit reached for requests","type":"requests","param":null,"code":"rate_limit_exceeded"}}"#,
        )
        .await;
        assert_eq!(limited.category, ErrorCategory::RateLimited);
        assert_eq!(
            limited.message,
            "OpenAI is rate limiting usage requests. Waiting before retrying."
        );
        let down = fail(ADMIN_KEY, 503, "upstream connect error").await;
        assert_eq!(down.category, ErrorCategory::Http5xx);
        assert_eq!(down.message, "OpenAI answered with HTTP 503.");
    }

    #[tokio::test]
    async fn an_answer_in_another_shape_is_a_decoding_error() {
        for body in [
            r#"{"object":"page","has_more":false}"#.to_string(),
            "<html>busy</html>".to_string(),
            page(
                vec![json!({"object": "bucket", "start_time": SEP_27, "end_time": SEP_27 + DAY})],
                None,
            ),
            page(
                vec![json!({"object": "bucket", "end_time": SEP_27 + DAY, "results": []})],
                None,
            ),
            page(
                vec![bucket(
                    SEP_27,
                    vec![json!({"amount": {"value": 1.0, "currency": "eur"}})],
                )],
                None,
            ),
            page(vec![bucket(SEP_27, vec![cost(json!("n/a"))])], None),
            page(vec![bucket(SEP_27, vec![json!({"amount": 0.06})])], None),
            page(vec![bucket(SEP_27, vec![json!("0.06")])], None),
            r#"{"object":"page","data":[],"has_more":true,"next_page":null}"#.to_string(),
        ] {
            let error = fail(ADMIN_KEY, 200, &body).await;
            assert_eq!(error.category, ErrorCategory::Decoding, "{body}");
            assert_eq!(
                error.message,
                "OpenAI returned usage data this version cannot read."
            );
        }
    }

    #[tokio::test]
    async fn a_secret_without_a_key_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), now());
        let error = OpenAi.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(
            error.message,
            "The OpenAI Admin API key is missing. Add it again in Accounts."
        );
        assert!(http.requests().is_empty());
    }

    #[test]
    fn connects_with_an_admin_key_and_discovers_nothing_on_disk() {
        let connection = OpenAi.connection();
        assert_eq!(connection.login_from, None);
        let help = connection.api_key.expect("an API key connection");
        assert_eq!(help.env, ["OPENAI_ADMIN_KEY"]);
        assert_eq!(
            help.url,
            "https://platform.openai.com/settings/organization/admin-keys"
        );
        assert!(help.fields.is_empty());
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path())
            .with_var("OPENAI_ADMIN_KEY", ADMIN_KEY)
            .with_var("OPENAI_API_KEY", PROJECT_KEY);
        assert!(OpenAi.discover(&roots).is_empty());
    }

    #[test]
    fn the_spend_rows_read_dollars_and_export_them_as_consumption() {
        let provider = Provider::new("openai@abc", "OpenAI");
        let descriptors = OpenAi.descriptors(&provider);
        let rows: Vec<(&str, &str)> = descriptors
            .iter()
            .map(|descriptor| (descriptor.id.as_str(), descriptor.metric_label.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("openai@abc.month", "Spend This Month"),
                ("openai@abc.today", "Today")
            ]
        );
        for descriptor in &descriptors {
            assert_eq!(descriptor.template.kind, MetricKind::Dollars);
            assert_eq!(
                descriptor.template.selection_kind,
                Some(MetricKind::Dollars)
            );
            assert_eq!(descriptor.template.limit, None);
            assert!(descriptor.template.is_usage_period);
        }
        let dollars = LimitResourceSource::Value {
            kind: MetricKind::Dollars,
            label: None,
        };
        let exports: Vec<_> = descriptors
            .iter()
            .flat_map(|descriptor| &descriptor.limit_resources)
            .map(|resource| {
                (
                    resource.key.as_str(),
                    resource.kind,
                    resource.unit.as_str(),
                    &resource.source,
                    resource.estimated,
                )
            })
            .collect();
        assert_eq!(
            exports,
            [
                (
                    "month",
                    LimitResourceKind::Consumption,
                    "usd",
                    &dollars,
                    false
                ),
                (
                    "today",
                    LimitResourceKind::Consumption,
                    "usd",
                    &dollars,
                    false
                ),
            ]
        );
    }
}
