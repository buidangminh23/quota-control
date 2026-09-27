//! OpenRouter: an API key saved in Accounts or set in `OPENROUTER_API_KEY` / `OPENROUTER_KEY`,
//! read the same way on Windows, macOS and Linux (OpenRouter has no app that keeps a login on this
//! computer, so nothing is discovered from files).
//!
//! `GET https://openrouter.ai/api/v1/key` gives the key's spend today, this week and this month
//! (UTC days, weeks from Monday), its optional spending cap and the account's free-model requests
//! for the day. `GET https://openrouter.ai/api/v1/credits` gives the credits bought and the balance
//! left, but OpenRouter answers it only for a management key: a regular key's card keeps its `/key`
//! rows and asks `/credits` again only every 12 hours. A management key cannot run models, so its
//! own spend is always zero and is left out.

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, Months, NaiveDate, NaiveTime, TimeZone, Utc};
use serde_json::{Value, json};
use uc_core::{
    ErrorCategory, HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind, MetricLine,
    Provider, ProviderLink, SimpleProviderError, WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct OpenRouter;

const NAME: &str = "OpenRouter";
const API: &str = "https://openrouter.ai/api/v1";

/// Memo key set when `/credits` refused this card's regular key.
const CREDITS_REFUSED: &str = "openrouter.creditsRefused";
const CREDITS_RETRY_HOURS: i64 = 12;

const CREDITS: &str = "Credits";
const BALANCE: &str = "Balance";
const TODAY: &str = "Today";
const THIS_WEEK: &str = "Spend This Week";
const THIS_MONTH: &str = "Spend This Month";
const KEY_LIMIT: &str = "Key Limit";
const FREE_REQUESTS: &str = "Free Requests";

const FREE_TIER: &str = "Free Tier";
const PAY_AS_YOU_GO: &str = "Pay As You Go";

#[async_trait]
impl Service for OpenRouter {
    fn id(&self) -> &'static str {
        "openrouter"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Status", "https://status.openrouter.ai"),
            ProviderLink::new("Activity", "https://openrouter.ai/activity"),
            ProviderLink::new("Credits", "https://openrouter.ai/settings/credits"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["OPENROUTER_API_KEY", "OPENROUTER_KEY"],
            url: "https://openrouter.ai/settings/keys",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let id = |suffix: &str| format!("{}.{suffix}", provider.id);
        let spend = |suffix: &str, title: &str| {
            WidgetDescriptor::values(
                id(suffix),
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
            WidgetDescriptor::bounded_dollars(
                id("credits"),
                provider,
                CREDITS,
                None,
                100.0,
                Some("purchased"),
                None,
            )
            .exporting_progress("credits", "usd"),
            WidgetDescriptor::dollar_balance(id("balance"), provider, BALANCE, None, "left")
                .exporting_limit(
                    "balance",
                    LimitResourceKind::Balance,
                    "usd",
                    LimitResourceSource::Value {
                        kind: MetricKind::Dollars,
                        label: None,
                    },
                    false,
                ),
            spend("today", TODAY),
            spend("week", THIS_WEEK),
            spend("month", THIS_MONTH),
            WidgetDescriptor::bounded_dollars(
                id("keyLimit"),
                provider,
                KEY_LIMIT,
                None,
                100.0,
                None,
                Some("spent"),
            )
            .exporting_progress("keyLimit", "usd"),
            WidgetDescriptor::bounded_count(
                id("freeRequests"),
                provider,
                FREE_REQUESTS,
                None,
                50.0,
                "requests",
                Some(lines::DAY_MS),
            )
            .exporting_progress("freeRequests", "requests"),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("No OpenRouter API key is saved. Add one in Accounts."))?;
        let current = ask(context, "key", key).await;
        match &current {
            Answer::Refused(401) => return Err(rejected()),
            Answer::Failed(error)
                if matches!(
                    error.category,
                    ErrorCategory::Network | ErrorCategory::RateLimited
                ) =>
            {
                return Err(error.clone());
            }
            _ => {}
        }
        let key_data = match &current {
            Answer::Data(data) => Some(data),
            _ => None,
        };
        let regular = key_data.is_some_and(|data| !is_management(data));
        let refused_lately = regular
            && context
                .memo
                .get(CREDITS_REFUSED, context.now)
                .await
                .is_some();
        let credits = if refused_lately {
            None
        } else {
            let answer = ask(context, "credits", key).await;
            if regular && matches!(answer, Answer::Refused(_)) {
                context
                    .memo
                    .put(
                        CREDITS_REFUSED,
                        json!(true),
                        Some(context.now + Duration::hours(CREDITS_RETRY_HOURS)),
                    )
                    .await;
            }
            Some(answer)
        };

        let mut found = Vec::new();
        if let Some(Answer::Data(data)) = &credits {
            found.extend(credit_lines(data));
        }
        if let Some(data) = key_data {
            found.extend(key_lines(data, context.now));
        }
        if !found.is_empty() {
            let plan = key_data
                .and_then(|data| value::flag(data, "/is_free_tier"))
                .map(|free| String::from(if free { FREE_TIER } else { PAY_AS_YOU_GO }));
            return Ok(Reading::new(plan, found));
        }
        if matches!(current, Answer::Refused(_)) && matches!(credits, Some(Answer::Refused(_))) {
            return Err(rejected());
        }
        Err([Some(&current), credits.as_ref()]
            .into_iter()
            .flatten()
            .find_map(|answer| match answer {
                Answer::Failed(error) => Some(error.clone()),
                _ => None,
            })
            .unwrap_or_else(|| http::decoding(NAME)))
    }
}

/// What one endpoint answered.
enum Answer {
    /// The `data` object of a successful answer.
    Data(Value),
    /// 401 or 403: OpenRouter would not let this key read the endpoint.
    Refused(u16),
    /// Any other failure, as the card error it stands for.
    Failed(SimpleProviderError),
}

async fn ask(context: &FetchContext<'_>, endpoint: &str, key: &str) -> Answer {
    let request = HttpRequest::get(format!("{API}/{endpoint}"))
        .bearer(key)
        .header("Accept", "application/json");
    let response = match http::send(context.http, request, NAME).await {
        Ok(response) => response,
        Err(error) => return Answer::Failed(error),
    };
    if matches!(response.status, 401 | 403) {
        return Answer::Refused(response.status);
    }
    if !response.is_success() {
        return Answer::Failed(http::status_error(&response, NAME));
    }
    match http::parse(&response, NAME) {
        Ok(mut body) => match body.get_mut("data").map(Value::take) {
            Some(data @ Value::Object(_)) => Answer::Data(data),
            _ => Answer::Failed(http::decoding(NAME)),
        },
        Err(error) => Answer::Failed(error),
    }
}

fn rejected() -> SimpleProviderError {
    http::invalid("OpenRouter rejected this API key. Check it at openrouter.ai/settings/keys.")
}

fn is_management(data: &Value) -> bool {
    value::flag(data, "/is_management_key")
        .or_else(|| value::flag(data, "/is_provisioning_key"))
        .unwrap_or(false)
}

/// The Credits meter (spent against bought) and the Balance left, from `/credits`. A real zero
/// balance is shown; an account that never bought credits has no meter.
fn credit_lines(data: &Value) -> Vec<MetricLine> {
    let (Some(bought), Some(spent)) = (
        value::number(data, "/total_credits"),
        value::number(data, "/total_usage"),
    ) else {
        return Vec::new();
    };
    let bought = bought.max(0.0);
    let spent = spent.max(0.0);
    let mut found = Vec::new();
    if bought > 0.0 {
        found.push(lines::dollars(CREDITS, spent, bought, None, None));
    }
    found.push(lines::dollar_value(BALANCE, (bought - spent).max(0.0)));
    found
}

/// The key's spend rows and spending cap (not for a management key), then the account's
/// free-model requests, from `/key`.
fn key_lines(data: &Value, now: DateTime<Utc>) -> Vec<MetricLine> {
    let mut found = Vec::new();
    if !is_management(data) {
        for (pointer, label) in [
            ("/usage_daily", TODAY),
            ("/usage_weekly", THIS_WEEK),
            ("/usage_monthly", THIS_MONTH),
        ] {
            if let Some(amount) = value::number(data, pointer) {
                found.push(lines::dollar_value(label, amount.max(0.0)));
            }
        }
        found.extend(key_limit(data, now));
    }
    found.extend(free_requests(data, now));
    found
}

/// What the key spent in its cap's current window against the cap. OpenRouter's own remaining
/// amount wins; without it the spend of the reset window (or the key's lifetime) is used.
fn key_limit(data: &Value, now: DateTime<Utc>) -> Option<MetricLine> {
    let limit = value::number(data, "/limit").filter(|limit| *limit > 0.0)?;
    let window = value::text(data, "/limit_reset").map(str::to_ascii_lowercase);
    let (spend, resets_at, period) = match window.as_deref() {
        Some("daily") => ("/usage_daily", Some(next_day(now)), Some(lines::DAY_MS)),
        Some("weekly") => ("/usage_weekly", Some(next_week(now)), Some(lines::WEEK_MS)),
        Some("monthly") => ("/usage_monthly", next_month(now), Some(lines::MONTH_MS)),
        _ => ("/usage", None, None),
    };
    let used = match value::number(data, "/limit_remaining") {
        Some(remaining) => limit - remaining.clamp(0.0, limit),
        None => value::number(data, spend)?.max(0.0),
    };
    Some(lines::dollars(KEY_LIMIT, used, limit, resets_at, period))
}

/// Free-model requests made today against the account's daily allowance (50, or 1,000 once $10 of
/// credits were bought), which renews at midnight UTC.
fn free_requests(data: &Value, now: DateTime<Utc>) -> Option<MetricLine> {
    let limit =
        value::number(data, "/free_model_daily_requests/limit").filter(|limit| *limit > 0.0)?;
    let used = value::number(data, "/free_model_daily_requests/used").or_else(|| {
        value::number(data, "/free_model_daily_requests/remaining")
            .map(|remaining| limit - remaining)
    })?;
    Some(lines::count(
        FREE_REQUESTS,
        used,
        limit,
        "requests",
        Some(next_day(now)),
        Some(lines::DAY_MS),
    ))
}

fn midnight(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_time(NaiveTime::MIN))
}

fn next_day(now: DateTime<Utc>) -> DateTime<Utc> {
    midnight(now.date_naive() + Duration::days(1))
}

/// The next Monday midnight UTC: OpenRouter weeks run Monday to Sunday.
fn next_week(now: DateTime<Utc>) -> DateTime<Utc> {
    let today = now.date_naive();
    let days = 7 - i64::from(today.weekday().num_days_from_monday());
    midnight(today + Duration::days(days))
}

fn next_month(now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let first = now.date_naive().with_day(1)?;
    first.checked_add_months(Months::new(1)).map(midnight)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Memo, Roots, Secret};
    use crate::testing::{Scripted, context_at, header};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uc_core::{HttpClient, HttpError, HttpResponse, ProgressFormat, SharedHttpClient};

    const KEY_URL: &str = "https://openrouter.ai/api/v1/key";
    const CREDITS_URL: &str = "https://openrouter.ai/api/v1/credits";

    /// A regular key with a weekly $20 cap, as `/key` answers it.
    const REGULAR: &str = r#"{"data":{
        "label":"sk-or-v1-abc...123",
        "limit":20,
        "limit_remaining":12.5,
        "limit_reset":"weekly",
        "include_byok_in_limit":false,
        "usage":42.75,
        "usage_daily":1.25,
        "usage_weekly":7.5,
        "usage_monthly":18.25,
        "byok_usage":0,"byok_usage_daily":0,"byok_usage_weekly":0,"byok_usage_monthly":0,
        "is_free_tier":false,
        "is_management_key":false,
        "is_provisioning_key":false,
        "rate_limit":{"requests":-1,"interval":"10s","note":"This field is deprecated and safe to ignore."},
        "expires_at":null,
        "free_model_daily_requests":{"limit":1000,"used":12,"remaining":988}
    }}"#;

    /// A management key: it cannot run models, so its own spend stays zero.
    const MANAGEMENT: &str = r#"{"data":{
        "label":"Monitoring",
        "limit":null,
        "limit_remaining":null,
        "limit_reset":null,
        "usage":0,"usage_daily":0,"usage_weekly":0,"usage_monthly":0,
        "is_free_tier":false,
        "is_management_key":true,
        "is_provisioning_key":true,
        "free_model_daily_requests":{"limit":1000,"used":40,"remaining":960}
    }}"#;

    const BALANCE_BODY: &str = r#"{"data":{"total_credits":100.5,"total_usage":25.75}}"#;
    const FORBIDDEN: &str =
        r#"{"error":{"code":403,"message":"Only management keys can perform this operation"}}"#;

    /// Wednesday 23 September 2026, 15:30 UTC.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 23, 15, 30, 0).unwrap()
    }

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
    }

    fn secret() -> Value {
        json!({"apiKey": "sk-or-v1-test"})
    }

    #[derive(Debug, PartialEq)]
    enum Seen {
        Row(String, f64),
        Meter(
            String,
            f64,
            f64,
            ProgressFormat,
            Option<DateTime<Utc>>,
            Option<i64>,
        ),
    }

    fn seen(lines: &[MetricLine]) -> Vec<Seen> {
        lines
            .iter()
            .map(|line| match line {
                MetricLine::Values(row) => {
                    assert_eq!(row.values.len(), 1);
                    assert_eq!(row.values[0].kind, MetricKind::Dollars);
                    Seen::Row(row.label.clone(), row.values[0].number)
                }
                MetricLine::Progress(meter) => Seen::Meter(
                    meter.label.clone(),
                    meter.used,
                    meter.limit,
                    meter.format.clone(),
                    meter.resets_at,
                    meter.period_duration_ms,
                ),
                other => panic!("unexpected line {other:?}"),
            })
            .collect()
    }

    fn requests_count() -> ProgressFormat {
        ProgressFormat::Count {
            suffix: "requests".into(),
        }
    }

    fn urls(http: &Scripted) -> Vec<String> {
        http.requests()
            .into_iter()
            .map(|request| request.url)
            .collect()
    }

    #[tokio::test]
    async fn a_regular_key_shows_its_spend_cap_and_free_requests_without_credits() {
        let http =
            Scripted::new()
                .on("GET", KEY_URL, 200, REGULAR)
                .on("GET", CREDITS_URL, 403, FORBIDDEN);
        let scope = context_at(&http, secret(), now());
        let reading = OpenRouter.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pay As You Go"));
        assert_eq!(reading.warning, None);
        assert_eq!(
            seen(&reading.lines),
            vec![
                Seen::Row("Today".into(), 1.25),
                Seen::Row("Spend This Week".into(), 7.5),
                Seen::Row("Spend This Month".into(), 18.25),
                Seen::Meter(
                    "Key Limit".into(),
                    7.5,
                    20.0,
                    ProgressFormat::Dollars,
                    Some(at(2026, 9, 28)),
                    Some(lines::WEEK_MS),
                ),
                Seen::Meter(
                    "Free Requests".into(),
                    12.0,
                    1000.0,
                    requests_count(),
                    Some(at(2026, 9, 24)),
                    Some(lines::DAY_MS),
                ),
            ]
        );
    }

    #[tokio::test]
    async fn requests_are_authorized_gets_with_the_key() {
        let http =
            Scripted::new()
                .on("GET", KEY_URL, 200, REGULAR)
                .on("GET", CREDITS_URL, 403, FORBIDDEN);
        let scope = context_at(&http, secret(), now());
        OpenRouter.fetch(&scope.context()).await.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        for (request, url) in requests.iter().zip([KEY_URL, CREDITS_URL]) {
            assert_eq!(request.method, "GET");
            assert_eq!(request.url, url);
            assert_eq!(
                header(request, "Authorization"),
                Some("Bearer sk-or-v1-test")
            );
            assert_eq!(header(request, "Accept"), Some("application/json"));
            assert_eq!(request.body, None);
            assert_eq!(request.timeout, HttpRequest::DEFAULT_TIMEOUT);
        }
    }

    #[tokio::test]
    async fn a_regular_key_asks_for_credits_again_only_every_12_hours() {
        let http =
            Scripted::new()
                .on("GET", KEY_URL, 200, REGULAR)
                .on("GET", CREDITS_URL, 403, FORBIDDEN);
        let shared = http.shared();
        let secret = Secret::new(secret());
        let memo = Memo::default();
        for hours in [0, 1, 13] {
            let context = FetchContext {
                secret: &secret,
                http: &shared,
                now: now() + Duration::hours(hours),
                memo: &memo,
            };
            assert_eq!(OpenRouter.fetch(&context).await.unwrap().lines.len(), 5);
        }
        assert_eq!(
            urls(&http),
            [KEY_URL, CREDITS_URL, KEY_URL, KEY_URL, CREDITS_URL]
        );
    }

    #[tokio::test]
    async fn a_management_key_shows_credits_and_balance_but_not_its_own_spend() {
        let http = Scripted::new().on("GET", KEY_URL, 200, MANAGEMENT).on(
            "GET",
            CREDITS_URL,
            200,
            BALANCE_BODY,
        );
        let shared = http.shared();
        let secret = Secret::new(secret());
        let memo = Memo::default();
        for hours in [0, 1] {
            let context = FetchContext {
                secret: &secret,
                http: &shared,
                now: now() + Duration::hours(hours),
                memo: &memo,
            };
            let reading = OpenRouter.fetch(&context).await.unwrap();
            assert_eq!(reading.plan.as_deref(), Some("Pay As You Go"));
            assert_eq!(
                seen(&reading.lines),
                vec![
                    Seen::Meter(
                        "Credits".into(),
                        25.75,
                        100.5,
                        ProgressFormat::Dollars,
                        None,
                        None,
                    ),
                    Seen::Row("Balance".into(), 74.75),
                    Seen::Meter(
                        "Free Requests".into(),
                        40.0,
                        1000.0,
                        requests_count(),
                        Some(at(2026, 9, 24)),
                        Some(lines::DAY_MS),
                    ),
                ]
            );
        }
        assert_eq!(urls(&http), [KEY_URL, CREDITS_URL, KEY_URL, CREDITS_URL]);
    }

    #[tokio::test]
    async fn an_account_that_never_bought_credits_shows_a_zero_balance_and_no_meter() {
        let http = Scripted::new().on("GET", KEY_URL, 200, MANAGEMENT).on(
            "GET",
            CREDITS_URL,
            200,
            r#"{"data":{"total_credits":0,"total_usage":0}}"#,
        );
        let scope = context_at(&http, secret(), now());
        let reading = OpenRouter.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines[0], lines::dollar_value("Balance", 0.0));
        assert!(reading.lines.iter().all(|line| line.label() != "Credits"));
    }

    #[tokio::test]
    async fn a_free_tier_key_without_a_cap_shows_only_its_spend() {
        let http = Scripted::new()
            .on(
                "GET",
                KEY_URL,
                200,
                r#"{"data":{"limit":null,"limit_remaining":null,"limit_reset":null,"usage":0,
                    "usage_daily":0,"usage_weekly":0,"usage_monthly":0,"is_free_tier":true,
                    "is_management_key":false}}"#,
            )
            .on("GET", CREDITS_URL, 403, FORBIDDEN);
        let scope = context_at(&http, secret(), now());
        let reading = OpenRouter.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Free Tier"));
        assert_eq!(
            seen(&reading.lines),
            vec![
                Seen::Row("Today".into(), 0.0),
                Seen::Row("Spend This Week".into(), 0.0),
                Seen::Row("Spend This Month".into(), 0.0),
            ]
        );
    }

    #[tokio::test]
    async fn a_rejected_key_is_reported_without_asking_for_credits() {
        let http = Scripted::new().on(
            "GET",
            KEY_URL,
            401,
            r#"{"error":{"code":401,"message":"User not found."}}"#,
        );
        let scope = context_at(&http, secret(), now());
        let error = OpenRouter.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(
            error.message,
            "OpenRouter rejected this API key. Check it at openrouter.ai/settings/keys."
        );
        assert!(!error.message.contains("sk-or-v1-test"));
        assert_eq!(urls(&http), [KEY_URL]);
    }

    #[tokio::test]
    async fn a_key_refused_by_both_endpoints_is_rejected() {
        let http =
            Scripted::new()
                .on("GET", KEY_URL, 403, "{}")
                .on("GET", CREDITS_URL, 403, FORBIDDEN);
        let scope = context_at(&http, secret(), now());
        let error = OpenRouter.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(urls(&http), [KEY_URL, CREDITS_URL]);
    }

    #[tokio::test]
    async fn rate_limiting_stops_the_refresh_at_once() {
        let http = Scripted::new().on(
            "GET",
            KEY_URL,
            429,
            r#"{"error":{"code":429,"message":"Rate limit exceeded"}}"#,
        );
        let scope = context_at(&http, secret(), now());
        let error = OpenRouter.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
        assert_eq!(urls(&http), [KEY_URL]);
    }

    #[tokio::test]
    async fn credits_still_show_when_the_key_endpoint_fails_or_refuses() {
        for (status, body) in [(502, "bad gateway"), (403, "{}")] {
            let http = Scripted::new().on("GET", KEY_URL, status, body).on(
                "GET",
                CREDITS_URL,
                200,
                BALANCE_BODY,
            );
            let scope = context_at(&http, secret(), now());
            let reading = OpenRouter.fetch(&scope.context()).await.unwrap();
            assert_eq!(reading.plan, None, "{status}");
            assert_eq!(
                seen(&reading.lines),
                vec![
                    Seen::Meter(
                        "Credits".into(),
                        25.75,
                        100.5,
                        ProgressFormat::Dollars,
                        None,
                        None,
                    ),
                    Seen::Row("Balance".into(), 74.75),
                ],
                "{status}"
            );
            assert_eq!(urls(&http), [KEY_URL, CREDITS_URL]);
        }
    }

    #[tokio::test]
    async fn a_rate_limited_credits_answer_keeps_the_key_rows_and_is_asked_again() {
        let http = Scripted::new().on("GET", KEY_URL, 200, REGULAR).on(
            "GET",
            CREDITS_URL,
            429,
            r#"{"error":{"code":429,"message":"Rate limit exceeded"}}"#,
        );
        let shared = http.shared();
        let secret = Secret::new(secret());
        let memo = Memo::default();
        for hours in [0, 1] {
            let context = FetchContext {
                secret: &secret,
                http: &shared,
                now: now() + Duration::hours(hours),
                memo: &memo,
            };
            let reading = OpenRouter.fetch(&context).await.unwrap();
            assert_eq!(reading.plan.as_deref(), Some("Pay As You Go"));
            assert_eq!(reading.lines.len(), 5);
        }
        assert_eq!(urls(&http), [KEY_URL, CREDITS_URL, KEY_URL, CREDITS_URL]);
    }

    /// A client whose every request times out, counting the attempts.
    #[derive(Default)]
    struct Offline(AtomicUsize);

    #[async_trait]
    impl HttpClient for Offline {
        async fn send(&self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(HttpError::Timeout)
        }
    }

    #[tokio::test]
    async fn a_network_failure_stops_the_refresh_at_once() {
        let offline = Arc::new(Offline::default());
        let http: SharedHttpClient = offline.clone();
        let secret = Secret::new(secret());
        let memo = Memo::default();
        let context = FetchContext {
            secret: &secret,
            http: &http,
            now: now(),
            memo: &memo,
        };
        let error = OpenRouter.fetch(&context).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Network);
        assert_eq!(
            error.message,
            "Cannot connect to OpenRouter. Check the connection and try again."
        );
        assert_eq!(offline.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_failing_key_endpoint_with_refused_credits_reports_the_failure() {
        let http = Scripted::new().on("GET", KEY_URL, 503, "unavailable").on(
            "GET",
            CREDITS_URL,
            403,
            FORBIDDEN,
        );
        let scope = context_at(&http, secret(), now());
        let error = OpenRouter.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Http5xx);
        assert_eq!(error.message, "OpenRouter answered with HTTP 503.");
    }

    #[tokio::test]
    async fn an_unreadable_key_answer_is_not_mistaken_for_a_bad_key() {
        let http = Scripted::new()
            .on("GET", KEY_URL, 200, r#"{"data":{}}"#)
            .on("GET", CREDITS_URL, 403, FORBIDDEN);
        let scope = context_at(&http, secret(), now());
        let error = OpenRouter.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Decoding);

        let http = Scripted::new()
            .on("GET", KEY_URL, 200, r#"{"limit":5}"#)
            .on("GET", CREDITS_URL, 200, r#"{"data":[]}"#);
        let scope = context_at(&http, secret(), now());
        let error = OpenRouter.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Decoding);

        let http = Scripted::new()
            .on("GET", KEY_URL, 200, "<html>Service warming up</html>")
            .on("GET", CREDITS_URL, 403, FORBIDDEN);
        let scope = context_at(&http, secret(), now());
        let error = OpenRouter.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Decoding);
        assert_eq!(
            error.message,
            "OpenRouter returned usage data this version cannot read."
        );
        assert_eq!(urls(&http), [KEY_URL, CREDITS_URL]);
    }

    #[test]
    fn the_deprecated_provisioning_flag_still_marks_a_management_key() {
        let provisioning = json!({
            "is_provisioning_key": true,
            "limit": 5,
            "limit_remaining": 5,
            "usage_daily": 0,
            "usage_weekly": 0,
            "usage_monthly": 0
        });
        assert!(is_management(&provisioning));
        assert!(key_lines(&provisioning, now()).is_empty());
        assert!(!is_management(&json!({"usage_daily": 0})));
    }

    #[tokio::test]
    async fn a_secret_without_a_key_sends_nothing() {
        let http = Scripted::new().on("GET", KEY_URL, 200, REGULAR);
        let scope = context_at(&http, json!({"apiKey": "  "}), now());
        let error = OpenRouter.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert!(http.requests().is_empty());
    }

    #[test]
    fn the_key_limit_follows_its_reset_window() {
        let daily = json!({"limit": 5, "limit_remaining": 3.5, "limit_reset": "daily"});
        assert_eq!(
            key_limit(&daily, now()),
            Some(lines::dollars(
                "Key Limit",
                1.5,
                5.0,
                Some(at(2026, 9, 24)),
                Some(lines::DAY_MS)
            ))
        );
        let december =
            json!({"limit": 10, "limit_reset": "monthly", "usage_monthly": 4, "usage": 30});
        assert_eq!(
            key_limit(
                &december,
                Utc.with_ymd_and_hms(2026, 12, 15, 8, 0, 0).unwrap()
            ),
            Some(lines::dollars(
                "Key Limit",
                4.0,
                10.0,
                Some(at(2027, 1, 1)),
                Some(lines::MONTH_MS)
            ))
        );
        let lifetime = json!({"limit": 10, "limit_reset": null, "usage": 12});
        assert_eq!(
            key_limit(&lifetime, now()),
            Some(lines::dollars("Key Limit", 12.0, 10.0, None, None))
        );
        let raised = json!({"limit": 10, "limit_remaining": 15, "limit_reset": "weekly"});
        let Some(MetricLine::Progress(meter)) = key_limit(&raised, now()) else {
            panic!("a key limit meter")
        };
        assert_eq!(meter.used, 0.0);
        let overspent = json!({"limit": 10, "limit_remaining": -2});
        let Some(MetricLine::Progress(meter)) = key_limit(&overspent, now()) else {
            panic!("a key limit meter")
        };
        assert_eq!(meter.used, 10.0);
        let window_without_spend = json!({"limit": 10, "limit_reset": "weekly", "usage": 3});
        assert_eq!(key_limit(&window_without_spend, now()), None);
        assert_eq!(key_limit(&json!({"limit": null, "usage": 3}), now()), None);
        assert_eq!(key_limit(&json!({"limit": 0, "usage": 3}), now()), None);
    }

    #[test]
    fn resets_fall_on_utc_midnights() {
        let monday = at(2026, 9, 28);
        assert_eq!(next_week(monday), at(2026, 10, 5));
        assert_eq!(next_week(at(2026, 9, 27)), monday);
        assert_eq!(
            next_day(Utc.with_ymd_and_hms(2026, 12, 31, 23, 59, 59).unwrap()),
            at(2027, 1, 1)
        );
        assert_eq!(next_month(at(2026, 1, 31)), Some(at(2026, 2, 1)));
    }

    #[test]
    fn free_requests_fall_back_to_the_remaining_count() {
        let data = json!({"free_model_daily_requests": {"limit": 50, "remaining": 38}});
        assert_eq!(
            free_requests(&data, now()),
            Some(lines::count(
                "Free Requests",
                12.0,
                50.0,
                "requests",
                Some(at(2026, 9, 24)),
                Some(lines::DAY_MS)
            ))
        );
        assert_eq!(
            free_requests(&json!({"free_model_daily_requests": {"limit": 0}}), now()),
            None
        );
    }

    #[tokio::test]
    async fn every_line_feeds_a_widget() {
        let provider = Provider::new("openrouter@abc", "OpenRouter");
        let descriptors = OpenRouter.descriptors(&provider);
        let ids: Vec<_> = descriptors.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "openrouter@abc.credits",
                "openrouter@abc.balance",
                "openrouter@abc.today",
                "openrouter@abc.week",
                "openrouter@abc.month",
                "openrouter@abc.keyLimit",
                "openrouter@abc.freeRequests",
            ]
        );
        let exports: Vec<_> = descriptors
            .iter()
            .flat_map(|d| d.limit_resources.iter().map(|r| r.key.as_str()))
            .collect();
        assert_eq!(exports, ["credits", "balance", "keyLimit", "freeRequests"]);
        let labels: Vec<_> = descriptors
            .iter()
            .map(|d| d.metric_label.as_str())
            .collect();
        for key in [REGULAR, MANAGEMENT] {
            let http = Scripted::new().on("GET", KEY_URL, 200, key).on(
                "GET",
                CREDITS_URL,
                200,
                BALANCE_BODY,
            );
            let scope = context_at(&http, secret(), now());
            let reading = OpenRouter.fetch(&scope.context()).await.unwrap();
            let fed: Vec<_> = reading.lines.iter().map(MetricLine::label).collect();
            assert!(fed.iter().all(|label| labels.contains(label)), "{fed:?}");
            if key == REGULAR {
                assert_eq!(
                    fed, labels,
                    "a regular key /credits accepts feeds every widget"
                );
            }
        }
    }

    #[test]
    fn it_connects_with_a_key_from_accounts_or_the_environment() {
        let connection = OpenRouter.connection();
        assert_eq!(connection.login_from, None);
        let help = connection.api_key.unwrap();
        assert_eq!(help.env, ["OPENROUTER_API_KEY", "OPENROUTER_KEY"]);
        assert_eq!(help.url, "https://openrouter.ai/settings/keys");
        assert!(help.fields.is_empty());
        let dir = tempfile::tempdir().unwrap();
        assert!(OpenRouter.discover(&Roots::under(dir.path())).is_empty());
    }
}
