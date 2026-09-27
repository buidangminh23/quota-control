//! Anthropic: what the organization spent on the Claude API this month and today, read with an
//! Admin API key.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `ANTHROPIC_ADMIN_KEY` environment variable or from a key saved in Quota Control. Anthropic shows
//! costs only to an Admin API key (`sk-ant-admin…`) or to a personal or service account key that is
//! not scoped to a workspace. A workspace key, the usual `ANTHROPIC_API_KEY`, is refused, so that
//! variable is not read.
//!
//! A refresh sends `GET https://api.anthropic.com/v1/organizations/cost_report` with the key in
//! `x-api-key` and `anthropic-version: 2023-06-01`, asking for one bucket per UTC day from the first
//! of the current month through today. One page holds up to 31 buckets, which covers any month;
//! `next_page` is still followed when Anthropic pages the answer. Each amount is a decimal string in
//! cents (`"123.45"` is $1.2345), so the rows add the amounts up and divide by 100. Priority Tier
//! usage is billed apart and is not in the report.

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, SecondsFormat, TimeZone, Utc};
use serde_json::Value;
use uc_core::{
    HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind, MetricLine, Provider,
    ProviderLink, SimpleProviderError, WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Anthropic;

const NAME: &str = "Anthropic";
const COST_REPORT: &str = "https://api.anthropic.com/v1/organizations/cost_report";
const API_VERSION: &str = "2023-06-01";
const KEY_URL: &str = "https://platform.claude.com/settings/admin-keys";

const SPEND_THIS_MONTH: &str = "Spend This Month";
const TODAY: &str = "Today";

/// Daily buckets asked for per page: Anthropic's maximum, which holds any month.
const DAYS_PER_PAGE: &str = "31";
/// Pages read in one refresh. The month fits one page; five pages still cover it at Anthropic's
/// default of seven days per page.
const MAX_PAGES: usize = 5;

const ADMIN_KEY: &str = "sk-ant-admin";
/// Claude.ai and Claude Code OAuth tokens, which the cost report never takes as a key.
const OAUTH_TOKENS: [&str; 2] = ["sk-ant-oat", "sk-ant-ort"];

const NO_KEY: &str = "No Anthropic Admin API key is saved. Add one in Accounts.";
const NEEDS_ADMIN_KEY: &str = "Anthropic shares costs only with an Admin API key (sk-ant-admin…). Create one at platform.claude.com/settings/admin-keys.";
const ADMIN_KEY_REFUSED: &str = "Anthropic refused this Admin API key; it may have expired or been disabled. Create a new one at platform.claude.com/settings/admin-keys.";

#[async_trait]
impl Service for Anthropic {
    fn id(&self) -> &'static str {
        "anthropic"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Status", "https://status.claude.com"),
            ProviderLink::new("Cost", "https://platform.claude.com/cost"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["ANTHROPIC_ADMIN_KEY"],
            url: KEY_URL,
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let spend = |suffix: &str, title: &str| {
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
        };
        vec![
            spend("spendThisMonth", SPEND_THIS_MONTH).exporting_limit(
                "spendThisMonth",
                LimitResourceKind::Consumption,
                "usd",
                LimitResourceSource::Value {
                    kind: MetricKind::Dollars,
                    label: None,
                },
                false,
            ),
            spend("today", TODAY),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context.secret.key().ok_or_else(|| http::invalid(NO_KEY))?;
        if OAUTH_TOKENS.iter().any(|prefix| has_prefix(key, prefix)) {
            return Err(http::invalid(NEEDS_ADMIN_KEY));
        }
        let days = Days::around(context.now);
        let mut spend = Spend::default();
        let mut page: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let body = cost_page(context, key, &days, page.as_deref()).await?;
            spend.add(&body, &days)?;
            match next_page(&body)? {
                None => return Ok(Reading::new(None, spend.lines())),
                Some(next) if page.as_deref() == Some(next.as_str()) => {
                    return Err(http::decoding(NAME));
                }
                Some(next) => page = Some(next),
            }
        }
        Err(http::decoding(NAME))
    }
}

/// One page of the cost report. A refused key is told apart by its kind: an Admin API key that
/// stopped working asks for a new one, any other key for an Admin API key.
async fn cost_page(
    context: &FetchContext<'_>,
    key: &str,
    days: &Days,
    page: Option<&str>,
) -> Result<Value, SimpleProviderError> {
    let response = http::send(
        context.http,
        HttpRequest::get(page_url(days, page))
            .header("x-api-key", key)
            .header("anthropic-version", API_VERSION)
            .header("Accept", "application/json"),
        NAME,
    )
    .await?;
    if matches!(response.status, 401 | 403) {
        return Err(if has_prefix(key, ADMIN_KEY) {
            http::expired(ADMIN_KEY_REFUSED)
        } else {
            http::invalid(NEEDS_ADMIN_KEY)
        });
    }
    if !response.is_success() {
        return Err(http::status_error(&response, NAME));
    }
    http::parse(&response, NAME)
}

fn page_url(days: &Days, page: Option<&str>) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query
        .append_pair("starting_at", &rfc3339(days.month))
        .append_pair("ending_at", &rfc3339(days.tomorrow))
        .append_pair("bucket_width", "1d")
        .append_pair("limit", DAYS_PER_PAGE);
    if let Some(page) = page {
        query.append_pair("page", page);
    }
    format!("{COST_REPORT}?{}", query.finish())
}

fn rfc3339(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The cursor of the next page, `None` on the last one. A page that says more follows without
/// naming a cursor cannot be continued, and a total without the rest would be short.
fn next_page(body: &Value) -> Result<Option<String>, SimpleProviderError> {
    if value::flag(body, "/has_more") != Some(true) {
        return Ok(None);
    }
    value::text(body, "/next_page")
        .map(|cursor| Some(cursor.to_string()))
        .ok_or_else(|| http::decoding(NAME))
}

/// Whether `key` starts with `prefix`, compared without copying the key.
fn has_prefix(key: &str, prefix: &str) -> bool {
    key.get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

/// The UTC days a refresh reads: the month so far, today last. Anthropic cuts its buckets at UTC
/// midnight.
struct Days {
    month: DateTime<Utc>,
    today: DateTime<Utc>,
    tomorrow: DateTime<Utc>,
}

impl Days {
    fn around(now: DateTime<Utc>) -> Self {
        let today = now.date_naive();
        Self {
            month: midnight(today.with_day(1).unwrap_or(today)),
            today: midnight(today),
            tomorrow: midnight(today + Duration::days(1)),
        }
    }
}

fn midnight(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_time(NaiveTime::MIN))
}

/// The dollars added up so far.
#[derive(Default)]
struct Spend {
    month: f64,
    /// Today's spend, once a bucket for today arrived.
    today: Option<f64>,
}

impl Spend {
    /// Adds a page's buckets that fall in `days`. A bucket or amount this version cannot read fails
    /// the refresh rather than leave a total short.
    fn add(&mut self, body: &Value, days: &Days) -> Result<(), SimpleProviderError> {
        let buckets = body
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| http::decoding(NAME))?;
        for bucket in buckets {
            let start = value::time(bucket, "/starting_at").ok_or_else(|| http::decoding(NAME))?;
            if start < days.month || start >= days.tomorrow {
                continue;
            }
            let results = bucket
                .get("results")
                .and_then(Value::as_array)
                .ok_or_else(|| http::decoding(NAME))?;
            let mut dollars = 0.0;
            for result in results.iter().filter(|result| in_dollars(result)) {
                let cents = value::number(result, "/amount").ok_or_else(|| http::decoding(NAME))?;
                dollars += cents / 100.0;
            }
            self.month += dollars;
            if start >= days.today {
                *self.today.get_or_insert(0.0) += dollars;
            }
        }
        Ok(())
    }

    /// The month's row, then today's when Anthropic had a bucket for it.
    fn lines(&self) -> Vec<MetricLine> {
        let mut found = vec![lines::dollar_value(SPEND_THIS_MONTH, self.month.max(0.0))];
        if let Some(today) = self.today {
            found.push(lines::dollar_value(TODAY, today.max(0.0)));
        }
        found
    }
}

/// Whether a cost item is in US dollars. Anthropic reports only USD today; an item without a
/// currency is read as USD.
fn in_dollars(result: &Value) -> bool {
    value::text(result, "/currency").is_none_or(|currency| currency.eq_ignore_ascii_case("USD"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Roots;
    use crate::testing::{Scripted, context_at, header};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const ADMIN: &str = "sk-ant-admin01-test-0123456789abcdef";
    const WORKSPACE: &str = "sk-ant-api03-test-0123456789abcdef";

    /// Thursday 3 September 2026, 10:00 UTC: the month so far is three daily buckets.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 3, 10, 0, 0).unwrap()
    }

    /// The first page's URL at `now()`: 1 September through today, 31 buckets per page.
    const FIRST_PAGE: &str = "https://api.anthropic.com/v1/organizations/cost_report?starting_at=2026-09-01T00%3A00%3A00Z&ending_at=2026-09-04T00%3A00%3A00Z&bucket_width=1d&limit=31";

    fn bucket(day: u32, results: Value) -> Value {
        json!({
            "starting_at": format!("2026-09-{day:02}T00:00:00Z"),
            "ending_at": format!("2026-09-{:02}T00:00:00Z", day + 1),
            "results": results,
        })
    }

    /// A cost item as Anthropic reports it without grouping: an amount in cents and its currency.
    fn cost(amount: &str, currency: &str) -> Value {
        json!({
            "amount": amount,
            "currency": currency,
            "context_window": null,
            "cost_type": null,
            "description": null,
            "inference_geo": null,
            "model": null,
            "service_tier": null,
            "token_type": null,
            "workspace_id": null,
        })
    }

    fn page(buckets: Vec<Value>, next: Option<&str>) -> String {
        json!({"data": buckets, "has_more": next.is_some(), "next_page": next}).to_string()
    }

    /// $12.50 on the 1st, nothing on the 2nd and $3.3125 so far today.
    fn month_so_far() -> String {
        page(
            vec![
                bucket(1, json!([cost("1250", "USD")])),
                bucket(2, json!([])),
                bucket(3, json!([cost("331.25", "USD")])),
            ],
            None,
        )
    }

    fn spend(month: f64, today: Option<f64>) -> Vec<MetricLine> {
        let mut rows = vec![lines::dollar_value("Spend This Month", month)];
        rows.extend(today.map(|today| lines::dollar_value("Today", today)));
        rows
    }

    async fn run(
        key: &str,
        answers: &[(u16, String)],
    ) -> (Scripted, Result<Reading, SimpleProviderError>) {
        let http = answers
            .iter()
            .fold(Scripted::new(), |http, (status, body)| {
                http.on("GET", COST_REPORT, *status, body)
            });
        let scope = context_at(&http, json!({ "apiKey": key }), now());
        let result = Anthropic.fetch(&scope.context()).await;
        (http, result)
    }

    async fn read(body: String) -> Reading {
        run(ADMIN, &[(200, body)]).await.1.unwrap()
    }

    async fn fail(key: &str, status: u16, body: &str) -> SimpleProviderError {
        run(key, &[(status, body.to_string())]).await.1.unwrap_err()
    }

    #[tokio::test]
    async fn adds_up_the_month_so_far_and_today_in_dollars() {
        let reading = read(month_so_far()).await;
        assert_eq!(reading, Reading::new(None, spend(15.8125, Some(3.3125))));
    }

    #[tokio::test]
    async fn sends_the_admin_key_and_api_version_for_the_month_so_far() {
        let (http, result) = run(ADMIN, &[(200, month_so_far())]).await;
        result.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, "GET");
        assert_eq!(request.url, FIRST_PAGE);
        assert_eq!(header(request, "x-api-key"), Some(ADMIN));
        assert_eq!(header(request, "anthropic-version"), Some("2023-06-01"));
        assert_eq!(header(request, "Accept"), Some("application/json"));
        assert_eq!(header(request, "Authorization"), None);
        assert_eq!(request.body, None);
        assert_eq!(request.timeout, HttpRequest::DEFAULT_TIMEOUT);
    }

    #[test]
    fn the_window_runs_from_the_first_of_the_month_through_today_in_utc() {
        let late = Days::around(Utc.with_ymd_and_hms(2026, 9, 27, 23, 59, 59).unwrap());
        assert_eq!(
            page_url(&late, None),
            "https://api.anthropic.com/v1/organizations/cost_report?starting_at=2026-09-01T00%3A00%3A00Z&ending_at=2026-09-28T00%3A00%3A00Z&bucket_width=1d&limit=31"
        );
        let new_year = Days::around(Utc.with_ymd_and_hms(2026, 12, 31, 23, 30, 0).unwrap());
        assert_eq!(
            page_url(&new_year, Some("page_MjAyNi0xMi0xNQ==")),
            "https://api.anthropic.com/v1/organizations/cost_report?starting_at=2026-12-01T00%3A00%3A00Z&ending_at=2027-01-01T00%3A00%3A00Z&bucket_width=1d&limit=31&page=page_MjAyNi0xMi0xNQ%3D%3D"
        );
        let first_day = Days::around(Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap());
        assert_eq!(
            (first_day.month, first_day.today, first_day.tomorrow),
            (
                Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
                Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
                Utc.with_ymd_and_hms(2026, 10, 2, 0, 0, 0).unwrap(),
            )
        );
    }

    #[tokio::test]
    async fn follows_next_page_until_the_last_page() {
        let cursor = "page_MjAyNi0wOS0wM1QwMDowMDowMFo=";
        let (http, result) = run(
            ADMIN,
            &[
                (
                    200,
                    page(
                        vec![
                            bucket(1, json!([cost("1250", "USD")])),
                            bucket(2, json!([])),
                        ],
                        Some(cursor),
                    ),
                ),
                (
                    200,
                    page(vec![bucket(3, json!([cost("331.25", "USD")]))], None),
                ),
            ],
        )
        .await;
        assert_eq!(result.unwrap().lines, spend(15.8125, Some(3.3125)));
        let urls: Vec<_> = http
            .requests()
            .into_iter()
            .map(|request| request.url)
            .collect();
        assert_eq!(
            urls,
            [
                FIRST_PAGE.to_string(),
                format!("{FIRST_PAGE}&page=page_MjAyNi0wOS0wM1QwMDowMDowMFo%3D"),
            ]
        );
    }

    #[tokio::test]
    async fn stops_paging_when_the_pages_do_not_end() {
        let endless: Vec<_> = (1..=6)
            .map(|index| {
                (
                    200,
                    page(Vec::new(), Some(format!("page_{index}").as_str())),
                )
            })
            .collect();
        let (http, result) = run(ADMIN, &endless).await;
        assert_eq!(result.unwrap_err().category, ErrorCategory::Decoding);
        assert_eq!(http.requests().len(), MAX_PAGES);

        let repeated = [
            (200, page(Vec::new(), Some("page_1"))),
            (200, page(Vec::new(), Some("page_1"))),
        ];
        let (http, result) = run(ADMIN, &repeated).await;
        assert_eq!(result.unwrap_err().category, ErrorCategory::Decoding);
        assert_eq!(http.requests().len(), 2);

        let (http, result) = run(
            ADMIN,
            &[(
                200,
                r#"{"data":[],"has_more":true,"next_page":null}"#.into(),
            )],
        )
        .await;
        assert_eq!(result.unwrap_err().category, ErrorCategory::Decoding);
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn without_a_bucket_for_today_only_the_month_is_shown() {
        let reading = read(page(
            vec![
                bucket(1, json!([cost("1250", "USD")])),
                bucket(2, json!([])),
            ],
            None,
        ))
        .await;
        assert_eq!(reading.lines, spend(12.5, None));
    }

    #[tokio::test]
    async fn ignores_days_outside_the_month_and_costs_in_other_currencies() {
        let reading = read(page(
            vec![
                json!({
                    "starting_at": "2026-08-31T00:00:00Z",
                    "ending_at": "2026-09-01T00:00:00Z",
                    "results": [cost("99999", "USD")],
                }),
                bucket(1, json!([cost("500", "USD"), cost("700", "EUR")])),
                bucket(
                    3,
                    json!([{"amount": "25", "currency": "usd"}, {"amount": "50"}]),
                ),
            ],
            None,
        ))
        .await;
        assert_eq!(reading.lines, spend(5.75, Some(0.75)));
    }

    #[tokio::test]
    async fn a_month_without_costs_shows_real_zeros() {
        let reading = read(page(
            vec![
                bucket(1, json!([])),
                bucket(2, json!([])),
                bucket(3, json!([])),
            ],
            None,
        ))
        .await;
        assert_eq!(reading, Reading::new(None, spend(0.0, Some(0.0))));
    }

    #[tokio::test]
    async fn a_refused_admin_key_asks_for_a_new_one() {
        for status in [401, 403] {
            let error = fail(
                ADMIN,
                status,
                r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
            )
            .await;
            assert_eq!(error.category, ErrorCategory::AuthExpired, "{status}");
            assert_eq!(
                error.message,
                "Anthropic refused this Admin API key; it may have expired or been disabled. Create a new one at platform.claude.com/settings/admin-keys."
            );
            assert!(!error.message.contains(ADMIN));
        }
    }

    #[tokio::test]
    async fn a_workspace_key_is_told_that_an_admin_key_is_needed() {
        for status in [401, 403] {
            let error = fail(
                WORKSPACE,
                status,
                r#"{"type":"error","error":{"type":"permission_error","message":"Your API key does not have permission to use the specified resource."}}"#,
            )
            .await;
            assert_eq!(error.category, ErrorCategory::AuthInvalid, "{status}");
            assert_eq!(
                error.message,
                "Anthropic shares costs only with an Admin API key (sk-ant-admin…). Create one at platform.claude.com/settings/admin-keys."
            );
            assert!(!error.message.contains(WORKSPACE));
        }
    }

    #[tokio::test]
    async fn oauth_tokens_and_missing_keys_send_nothing() {
        for (secret, message) in [
            (
                json!({"apiKey": "sk-ant-oat01-test-token"}),
                NEEDS_ADMIN_KEY,
            ),
            (
                json!({"apiKey": "SK-ANT-ORT01-test-token"}),
                NEEDS_ADMIN_KEY,
            ),
            (json!({"apiKey": "   "}), NO_KEY),
            (json!({}), NO_KEY),
        ] {
            let http = Scripted::new();
            let scope = context_at(&http, secret, now());
            let error = Anthropic.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthInvalid);
            assert_eq!(error.message, message);
            assert!(http.requests().is_empty());
        }
    }

    #[tokio::test]
    async fn rate_limits_and_server_errors_keep_their_categories() {
        let limited = fail(
            ADMIN,
            429,
            r#"{"type":"error","error":{"type":"rate_limit_error","message":"Rate limited"}}"#,
        )
        .await;
        assert_eq!(limited.category, ErrorCategory::RateLimited);
        assert_eq!(
            limited.message,
            "Anthropic is rate limiting usage requests. Waiting before retrying."
        );
        let overloaded = fail(
            ADMIN,
            529,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        )
        .await;
        assert_eq!(overloaded.category, ErrorCategory::Http5xx);
        assert_eq!(overloaded.message, "Anthropic answered with HTTP 529.");
    }

    #[tokio::test]
    async fn an_answer_this_version_cannot_read_is_a_decoding_error() {
        for body in [
            "{}",
            "<html>busy</html>",
            r#"{"data":[{"ending_at":"2026-09-02T00:00:00Z","results":[]}],"has_more":false}"#,
            r#"{"data":[{"starting_at":"2026-09-01T00:00:00Z"}],"has_more":false}"#,
            r#"{"data":[{"starting_at":"2026-09-01T00:00:00Z","results":[{"amount":"n/a","currency":"USD"}]}],"has_more":false}"#,
        ] {
            let error = fail(ADMIN, 200, body).await;
            assert_eq!(error.category, ErrorCategory::Decoding, "{body}");
            assert_eq!(
                error.message,
                "Anthropic returned usage data this version cannot read."
            );
        }
    }

    #[test]
    fn connects_with_an_admin_key_only() {
        let connection = Anthropic.connection();
        assert_eq!(connection.login_from, None);
        let help = connection.api_key.expect("an API key connection");
        assert_eq!(help.env, ["ANTHROPIC_ADMIN_KEY"]);
        assert_eq!(help.url, "https://platform.claude.com/settings/admin-keys");
        assert!(help.fields.is_empty());
        let dir = tempfile::tempdir().unwrap();
        assert!(Anthropic.discover(&Roots::under(dir.path())).is_empty());
        assert!(!Anthropic.starts_hidden());
        assert_eq!(
            Anthropic.links(),
            [
                ProviderLink::new("Status", "https://status.claude.com"),
                ProviderLink::new("Cost", "https://platform.claude.com/cost"),
            ]
        );
    }

    #[test]
    fn the_spend_rows_read_the_lines_the_fetch_writes() {
        let provider = Provider::new("anthropic@abc", "Anthropic");
        let descriptors = Anthropic.descriptors(&provider);
        let rows: Vec<_> = descriptors
            .iter()
            .map(|descriptor| {
                (
                    descriptor.id.as_str(),
                    descriptor.metric_label.as_str(),
                    descriptor.template.kind,
                    descriptor.template.selection_kind,
                    descriptor.template.limit,
                    descriptor.template.is_usage_period,
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "anthropic@abc.spendThisMonth",
                    "Spend This Month",
                    MetricKind::Dollars,
                    Some(MetricKind::Dollars),
                    None,
                    true
                ),
                (
                    "anthropic@abc.today",
                    "Today",
                    MetricKind::Dollars,
                    Some(MetricKind::Dollars),
                    None,
                    true
                ),
            ]
        );
        let labels: Vec<_> = spend(1.0, Some(1.0))
            .iter()
            .map(|line| line.label().to_string())
            .collect();
        assert_eq!(labels, ["Spend This Month", "Today"]);
        let month = &descriptors[0].limit_resources;
        assert_eq!(month.len(), 1);
        assert_eq!(month[0].key, "spendThisMonth");
        assert_eq!(month[0].kind, LimitResourceKind::Consumption);
        assert_eq!(month[0].unit, "usd");
        assert_eq!(
            month[0].source,
            LimitResourceSource::Value {
                kind: MetricKind::Dollars,
                label: None
            }
        );
        assert!(!month[0].estimated);
        assert!(descriptors[1].limit_resources.is_empty());
    }
}
