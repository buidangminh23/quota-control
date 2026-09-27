//! Helmcode: the per-model token quotas of a Helmcode Cloud or NaN Builders dashboard session, and
//! the prepaid balance of a Helmcode Cloud account.
//!
//! The dashboards sign in with a browser session only; their inference API keys cannot read usage.
//! Nothing is read from disk on Windows, macOS or Linux: the user pastes the whole Cookie header of
//! a signed-in dashboard request, and names the dashboard it belongs to (cloud.helmcode.com unless
//! NaN Builders is named). The header is sent only to that dashboard's API, never to the other one.
//!
//! A refresh sends `GET https://cloud-api.<dashboard>/api/usage/quota`, then `GET /api/billing` to
//! learn whether the subscription is premium and, on Helmcode Cloud only, `GET /api/billing/credits`
//! for the prepaid balance; each carries the Cookie header with the dashboard's own Origin and
//! Referer. The billing requests are best effort: when they fail, the quota still shows, without
//! the premium rows or the balance.
//!
//! Every model with a positive cap becomes a row of tokens used, the busiest first. A rolling tier
//! (a model with `windowHours`) shows only on a premium subscription, as the dashboard does, and
//! is labelled with its window. A row resets at the model's `periodEnd`, or on the first day of the
//! month after the quota's `periodStart`. Models without a cap follow with the tokens they used.

use std::collections::HashSet;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use serde_json::Value;
use uc_core::{
    HttpRequest, MetricLine, MetricValue, Provider, ProviderLink, SimpleProviderError,
    WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, KeyFormat, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Helmcode;

const NAME: &str = "Helmcode";
const QUOTA_PATH: &str = "/api/usage/quota";
const BILLING_PATH: &str = "/api/billing";
const CREDITS_PATH: &str = "/api/billing/credits";
const BALANCE: &str = "Balance";
const CREDIT_TOKENS: &str = "Credit-funded tokens";
const TOKENS: &str = "tokens";
const PREMIUM: &str = "Premium";
const MISSING: &str = "The Helmcode Cookie header is missing. Add it again in Accounts.";
const EXPIRED: &str =
    "The Helmcode dashboard session expired. Sign in again and paste a fresh Cookie header.";
const UNKNOWN_DASHBOARD: &str =
    "The Helmcode dashboard must be cloud.helmcode.com or cloud.nan.builders.";
/// How long a best-effort billing request may take before the quota shows without it.
const BILLING_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest rolling window a model may declare: a year.
const MAX_WINDOW_HOURS: f64 = 8_760.0;
/// The largest whole number a JSON number carries exactly.
const MAX_WHOLE: f64 = 9_007_199_254_740_991.0;
/// The longest model name shown as a row label.
const MAX_LABEL_CHARS: usize = 80;

#[async_trait]
impl Service for Helmcode {
    fn id(&self) -> &'static str {
        "helmcode"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Helmcode Cloud", "https://cloud.helmcode.com/dashboard"),
            ProviderLink::new("NaN Builders", "https://cloud.nan.builders/dashboard"),
        ]
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
            url: "https://cloud.helmcode.com/dashboard",
            fields: &[(
                "tenant",
                "Dashboard (default: cloud.helmcode.com, or cloud.nan.builders)",
            )],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![WidgetDescriptor::dollar_balance(
            format!("{}.balance", provider.id),
            provider,
            BALANCE,
            None,
            "left",
        )]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let cookie = context.secret.key().ok_or_else(|| http::invalid(MISSING))?;
        let dashboard = Dashboard::from_field(context.secret.str("/tenant"))
            .ok_or_else(|| http::invalid(UNKNOWN_DASHBOARD))?;
        let response =
            http::send(context.http, dashboard.request(QUOTA_PATH, cookie), NAME).await?;
        if (300..400).contains(&response.status) || matches!(response.status, 401 | 403) {
            return Err(http::expired(EXPIRED));
        }
        if !response.is_success() {
            return Err(http::status_error(&response, NAME));
        }
        let quota =
            Quota::read(&http::parse(&response, NAME)?).ok_or_else(|| http::decoding(NAME))?;
        let premium = best_effort(context, dashboard, BILLING_PATH, cookie)
            .await
            .and_then(|answer| {
                answer
                    .pointer("/subscription/premium")
                    .and_then(Value::as_bool)
            })
            == Some(true);
        let mut rows = quota.lines(premium);
        if dashboard == Dashboard::Cloud {
            let credits = best_effort(context, dashboard, CREDITS_PATH, cookie).await;
            if let Some(amount) = credits.as_ref().and_then(balance) {
                rows.insert(0, lines::values(BALANCE, vec![amount]));
            }
        }
        Ok(Reading::new(premium.then(|| PREMIUM.to_string()), rows))
    }
}

/// The two dashboards Helmcode runs, each with its own sessions and its own API host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dashboard {
    Cloud,
    NanBuilders,
}

impl Dashboard {
    /// The dashboard a saved account names: Helmcode Cloud when the field is empty, NaN Builders
    /// when it names that one by host, address or name. Anything else is refused rather than
    /// guessed, so a session never reaches the other dashboard.
    fn from_field(field: Option<&str>) -> Option<Self> {
        let text = field.unwrap_or_default().trim().to_ascii_lowercase();
        let host = text
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or_default();
        let host = host
            .strip_prefix("cloud-api.")
            .or_else(|| host.strip_prefix("cloud."))
            .unwrap_or(host);
        match host {
            "" | "helmcode" | "helmcode.com" | "helmcode cloud" => Some(Self::Cloud),
            "nan" | "nanbuilders" | "nan builders" | "nan.builders" => Some(Self::NanBuilders),
            _ => None,
        }
    }

    fn domain(self) -> &'static str {
        match self {
            Self::Cloud => "helmcode.com",
            Self::NanBuilders => "nan.builders",
        }
    }

    /// A dashboard API request carrying the session, from the dashboard's own page.
    fn request(self, path: &str, cookie: &str) -> HttpRequest {
        let origin = format!("https://cloud.{}", self.domain());
        HttpRequest::get(format!("https://cloud-api.{}{path}", self.domain()))
            .header("Accept", "application/json")
            .header("Cookie", cookie)
            .header("Referer", format!("{origin}/dashboard"))
            .header("Origin", origin)
    }
}

/// A best-effort billing answer: `None` when the request fails, is refused or is not a JSON object,
/// so billing trouble never hides the quota.
async fn best_effort(
    context: &FetchContext<'_>,
    dashboard: Dashboard,
    path: &str,
    cookie: &str,
) -> Option<Value> {
    let request = dashboard.request(path, cookie).timeout(BILLING_TIMEOUT);
    let response = http::send(context.http, request, NAME).await.ok()?;
    if response.status != 200 {
        return None;
    }
    http::parse(&response, NAME).ok().filter(Value::is_object)
}

/// The prepaid balance of a Helmcode Cloud account: `balanceMicros` millionths of its currency
/// (euros when none is named), never below zero. `None` when either field cannot be read.
fn balance(credits: &Value) -> Option<MetricValue> {
    let micros = credits.get("balanceMicros").and_then(Value::as_f64)?;
    if micros.fract() != 0.0 || micros.abs() > MAX_WHOLE {
        return None;
    }
    let currency = match credits.get("currency") {
        None | Some(Value::Null) => "EUR".to_string(),
        Some(currency) => currency.as_str()?.to_ascii_uppercase(),
    };
    if currency.len() != 3 || !currency.bytes().all(|letter| letter.is_ascii_uppercase()) {
        return None;
    }
    let amount = (micros / 1e6).max(0.0);
    Some(match currency.as_str() {
        "USD" => MetricValue::dollars(amount),
        _ => MetricValue::count(amount, currency),
    })
}

/// The quota answer: when the period began and each model's allowance.
struct Quota {
    period_start: Option<DateTime<Utc>>,
    fallback_reset: Option<DateTime<Utc>>,
    models: Vec<Model>,
}

/// One model of the quota: its cap and use in tokens, and its rolling window when it has one.
struct Model {
    name: String,
    cap: f64,
    used: f64,
    credit: f64,
    window_hours: Option<f64>,
    resets_at: Option<DateTime<Utc>>,
}

impl Quota {
    /// The quota, or `None` when its shape changed: it needs `periodStart` and a `models` list whose
    /// every entry names the model and gives its cap and the tokens used as whole numbers, so a
    /// renamed field shows as a version problem rather than as an empty card.
    fn read(body: &Value) -> Option<Self> {
        let start = body.get("periodStart")?.as_str()?;
        let models = body
            .get("models")?
            .as_array()?
            .iter()
            .map(Model::read)
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            period_start: value::time(body, "/periodStart"),
            fallback_reset: month_after(start),
            models,
        })
    }

    /// The rows of an account: capped models, the busiest first, with rolling tiers only on a
    /// premium subscription; then the credit-funded tokens, when there are any; then the models
    /// without a cap, by tokens used.
    fn lines(&self, premium: bool) -> Vec<MetricLine> {
        let mut capped: Vec<&Model> = self
            .models
            .iter()
            .filter(|model| model.cap > 0.0 && (model.window_hours.is_none() || premium))
            .collect();
        capped.sort_by(|left, right| {
            (right.used / right.cap)
                .total_cmp(&(left.used / left.cap))
                .then_with(|| left.name.cmp(&right.name))
        });
        let mut uncapped: Vec<&Model> = self
            .models
            .iter()
            .filter(|model| model.cap <= 0.0 && model.window_hours.is_none())
            .collect();
        uncapped.sort_by(|left, right| {
            right
                .used
                .total_cmp(&left.used)
                .then_with(|| left.name.cmp(&right.name))
        });
        let mut labels = HashSet::new();
        let mut rows = Vec::new();
        for model in capped {
            let label = model.label();
            if labels.insert(label.clone()) {
                let resets_at = model.resets_at.or(self.fallback_reset);
                rows.push(lines::count(
                    &label,
                    model.used,
                    model.cap,
                    TOKENS,
                    resets_at,
                    self.period_ms(model, resets_at),
                ));
            }
        }
        let credit: f64 = self.models.iter().map(|model| model.credit).sum();
        if credit > 0.0 {
            rows.push(lines::count_value(CREDIT_TOKENS, credit, TOKENS));
        }
        for model in uncapped {
            if labels.insert(model.name.clone()) {
                rows.push(lines::count_value(&model.name, model.used, TOKENS));
            }
        }
        rows
    }

    /// A row's pace window: the length of the quota period. A rolling tier's reset is the end of
    /// its allowance, days after its window closes, so it gets no pace.
    fn period_ms(&self, model: &Model, resets_at: Option<DateTime<Utc>>) -> Option<i64> {
        if model.window_hours.is_some() {
            return None;
        }
        let length = (resets_at? - self.period_start?).num_milliseconds();
        Some(if length > 0 { length } else { lines::MONTH_MS })
    }
}

impl Model {
    fn read(entry: &Value) -> Option<Self> {
        let name = entry.get("model")?.as_str()?.trim();
        if name.is_empty()
            || name.chars().count() > MAX_LABEL_CHARS
            || name.chars().any(char::is_control)
        {
            return None;
        }
        let window_hours = match entry.get("windowHours") {
            None | Some(Value::Null) => None,
            Some(hours) => {
                Some(whole(hours).filter(|hours| (1.0..=MAX_WINDOW_HOURS).contains(hours))?)
            }
        };
        let credit = match entry.get("creditTokens") {
            None | Some(Value::Null) => 0.0,
            Some(credit) => whole(credit)?,
        };
        Some(Self {
            name: name.to_string(),
            cap: whole(entry.get("cap")?)?,
            used: whole(entry.get("tokensUsed")?)?,
            credit,
            window_hours,
            resets_at: value::time(entry, "/periodEnd"),
        })
    }

    /// The row label: the model's name, and a rolling tier's window after it ("GLM-4.6 (5h)").
    fn label(&self) -> String {
        match self.window_hours {
            Some(hours) => format!("{} ({hours}h)", self.name),
            None => self.name.clone(),
        }
    }
}

/// A JSON number holding a whole, non-negative count, as the dashboard sends token counts.
fn whole(number: &Value) -> Option<f64> {
    let number = number.as_f64()?;
    (number >= 0.0 && number.fract() == 0.0 && number <= MAX_WHOLE).then_some(number)
}

/// The first instant of the month after the date `start` begins with ("2026-09-01" gives
/// 2026-10-01 00:00 UTC): the reset of a model whose quota names no end.
fn month_after(start: &str) -> Option<DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(start.get(..10)?, "%Y-%m-%d").ok()?;
    let (year, month) = match date.month() {
        12 => (date.year() + 1, 1),
        month => (date.year(), month + 1),
    };
    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0).single()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const COOKIE: &str = "session=abc; theme=dark";
    const CLOUD_QUOTA: &str = "https://cloud-api.helmcode.com/api/usage/quota";
    const CLOUD_BILLING: &str = "https://cloud-api.helmcode.com/api/billing";
    const CLOUD_CREDITS: &str = "https://cloud-api.helmcode.com/api/billing/credits";
    const NAN_QUOTA: &str = "https://cloud-api.nan.builders/api/usage/quota";
    const NAN_BILLING: &str = "https://cloud-api.nan.builders/api/billing";
    const QUOTA: &str = r#"{
        "periodStart": "2026-09-01T00:00:00Z",
        "models": [
            {"model": "GLM-4.6", "cap": 1000000, "tokensUsed": 250000, "creditTokens": 5000,
             "periodEnd": "2026-10-01T00:00:00Z"},
            {"model": "Kimi K2", "cap": 200000, "tokensUsed": 150000,
             "periodEnd": "2026-10-01T00:00:00Z"},
            {"model": "GLM-4.6", "cap": 40000, "tokensUsed": 36000, "windowHours": 5,
             "periodEnd": "2026-10-04T19:25:48Z"},
            {"model": "Qwen3 Coder", "cap": 0, "tokensUsed": 7000}
        ]
    }"#;
    const PREMIUM_BILLING: &str = r#"{"subscription": {"premium": true, "plan": "pro"}}"#;
    const BASIC_BILLING: &str = r#"{"subscription": {"premium": false}}"#;
    const CREDITS: &str = r#"{"balanceMicros": 12500000, "currency": "eur"}"#;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn at(text: &str) -> Option<DateTime<Utc>> {
        Some(DateTime::parse_from_rfc3339(text).unwrap().to_utc())
    }

    async fn read(http: &Scripted, secret: Value) -> Result<Reading, SimpleProviderError> {
        let scope = context_at(http, secret, now());
        Helmcode.fetch(&scope.context()).await
    }

    fn cloud(billing: &str) -> Scripted {
        Scripted::new()
            .on("GET", CLOUD_QUOTA, 200, QUOTA)
            .on("GET", CLOUD_BILLING, 200, billing)
            .on("GET", CLOUD_CREDITS, 200, CREDITS)
    }

    #[tokio::test]
    async fn a_premium_cloud_session_shows_the_balance_then_each_capped_model_busiest_first() {
        let http = cloud(PREMIUM_BILLING);
        let reading = read(&http, json!({ "apiKey": COOKIE })).await.unwrap();
        let september = 30 * lines::DAY_MS;
        assert_eq!(reading.plan.as_deref(), Some("Premium"));
        assert_eq!(
            reading.lines,
            vec![
                lines::values("Balance", vec![MetricValue::count(12.5, "EUR")]),
                lines::count(
                    "GLM-4.6 (5h)",
                    36_000.0,
                    40_000.0,
                    "tokens",
                    at("2026-10-04T19:25:48Z"),
                    None,
                ),
                lines::count(
                    "Kimi K2",
                    150_000.0,
                    200_000.0,
                    "tokens",
                    at("2026-10-01T00:00:00Z"),
                    Some(september),
                ),
                lines::count(
                    "GLM-4.6",
                    250_000.0,
                    1_000_000.0,
                    "tokens",
                    at("2026-10-01T00:00:00Z"),
                    Some(september),
                ),
                lines::count_value("Credit-funded tokens", 5_000.0, "tokens"),
                lines::count_value("Qwen3 Coder", 7_000.0, "tokens"),
            ]
        );
    }

    #[tokio::test]
    async fn every_request_carries_the_session_with_the_dashboards_own_origin() {
        let http = cloud(PREMIUM_BILLING);
        read(&http, json!({ "apiKey": COOKIE })).await.unwrap();
        let requests = http.requests();
        let urls: Vec<&str> = requests
            .iter()
            .map(|request| request.url.as_str())
            .collect();
        assert_eq!(urls, [CLOUD_QUOTA, CLOUD_BILLING, CLOUD_CREDITS]);
        for request in &requests {
            assert_eq!(request.method, "GET");
            assert_eq!(header(request, "Cookie"), Some(COOKIE));
            assert_eq!(
                header(request, "Origin"),
                Some("https://cloud.helmcode.com")
            );
            assert_eq!(
                header(request, "Referer"),
                Some("https://cloud.helmcode.com/dashboard")
            );
            assert_eq!(request.body, None);
        }
    }

    #[tokio::test]
    async fn a_nan_builders_session_stays_on_its_own_dashboard_which_has_no_balance() {
        let http = Scripted::new().on("GET", NAN_QUOTA, 200, QUOTA).on(
            "GET",
            NAN_BILLING,
            200,
            PREMIUM_BILLING,
        );
        let reading = read(
            &http,
            json!({ "apiKey": COOKIE, "tenant": "https://cloud.nan.builders/dashboard" }),
        )
        .await
        .unwrap();
        assert!(reading.lines.iter().all(|line| line.label() != "Balance"));
        let requests = http.requests();
        let urls: Vec<&str> = requests
            .iter()
            .map(|request| request.url.as_str())
            .collect();
        assert_eq!(urls, [NAN_QUOTA, NAN_BILLING]);
        assert_eq!(
            header(&requests[0], "Origin"),
            Some("https://cloud.nan.builders")
        );
        assert_eq!(
            header(&requests[0], "Referer"),
            Some("https://cloud.nan.builders/dashboard")
        );
    }

    #[tokio::test]
    async fn rolling_tiers_show_only_when_billing_says_the_subscription_is_premium() {
        for billing in [
            BASIC_BILLING,
            "{}",
            "not json",
            r#"{"subscription":{"premium":"true"}}"#,
        ] {
            let reading = read(&cloud(billing), json!({ "apiKey": COOKIE }))
                .await
                .unwrap();
            let labels: Vec<&str> = reading.lines.iter().map(MetricLine::label).collect();
            assert_eq!(
                labels,
                [
                    "Balance",
                    "Kimi K2",
                    "GLM-4.6",
                    "Credit-funded tokens",
                    "Qwen3 Coder"
                ],
                "{billing}"
            );
            assert_eq!(reading.plan, None);
        }
    }

    #[tokio::test]
    async fn billing_failures_never_hide_the_quota() {
        let http = Scripted::new()
            .on("GET", CLOUD_QUOTA, 200, QUOTA)
            .on("GET", CLOUD_BILLING, 503, "")
            .on("GET", CLOUD_CREDITS, 401, "{}");
        let reading = read(&http, json!({ "apiKey": COOKIE })).await.unwrap();
        let labels: Vec<&str> = reading.lines.iter().map(MetricLine::label).collect();
        assert_eq!(
            labels,
            ["Kimi K2", "GLM-4.6", "Credit-funded tokens", "Qwen3 Coder"]
        );
        assert_eq!(http.requests().len(), 3);
    }

    #[tokio::test]
    async fn a_model_without_an_end_resets_on_the_first_of_the_next_month() {
        let quota = r#"{"periodStart": "2026-12-01", "models": [
            {"model": "GLM-4.6", "cap": 100, "tokensUsed": 10}]}"#;
        let http = Scripted::new()
            .on("GET", CLOUD_QUOTA, 200, quota)
            .on("GET", CLOUD_BILLING, 200, BASIC_BILLING)
            .on("GET", CLOUD_CREDITS, 404, "");
        let reading = read(&http, json!({ "apiKey": COOKIE })).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![lines::count(
                "GLM-4.6",
                10.0,
                100.0,
                "tokens",
                at("2027-01-01T00:00:00Z"),
                Some(31 * lines::DAY_MS),
            )]
        );
    }

    #[tokio::test]
    async fn a_refused_or_redirected_session_reads_as_expired_and_sends_nothing_more() {
        for status in [302, 401, 403] {
            let http = Scripted::new().on("GET", CLOUD_QUOTA, status, "{}");
            let error = read(&http, json!({ "apiKey": COOKIE })).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthExpired, "{status}");
            assert_eq!(error.message, EXPIRED);
            assert_eq!(http.requests().len(), 1);
        }
    }

    #[tokio::test]
    async fn rate_limits_and_outages_keep_their_own_errors() {
        for (status, category) in [
            (429, ErrorCategory::RateLimited),
            (408, ErrorCategory::Http4xx),
            (503, ErrorCategory::Http5xx),
        ] {
            let http = Scripted::new().on("GET", CLOUD_QUOTA, status, "");
            let error = read(&http, json!({ "apiKey": COOKIE })).await.unwrap_err();
            assert_eq!(error.category, category, "{status}");
            assert_eq!(http.requests().len(), 1);
        }
    }

    #[tokio::test]
    async fn a_changed_quota_shape_is_a_version_problem_and_asks_for_no_billing() {
        for body in [
            "<html>Sign in</html>",
            r#"{"models": []}"#,
            r#"{"periodStart": "2026-09-01", "models": {}}"#,
            r#"{"periodStart": "2026-09-01", "models": [{"cap": 1, "tokensUsed": 0}]}"#,
            r#"{"periodStart": "2026-09-01", "models": [{"model": " ", "cap": 1, "tokensUsed": 0}]}"#,
            r#"{"periodStart": "2026-09-01", "models": [{"model": "A", "cap": -1, "tokensUsed": 0}]}"#,
            r#"{"periodStart": "2026-09-01", "models": [{"model": "A", "cap": 1.5, "tokensUsed": 0}]}"#,
            r#"{"periodStart": "2026-09-01", "models": [{"model": "A", "cap": "1", "tokensUsed": 0}]}"#,
            r#"{"periodStart": "2026-09-01", "models": [{"model": "A", "cap": 1, "tokensUsed": 0, "windowHours": 0}]}"#,
            r#"{"periodStart": "2026-09-01", "models": [{"model": "A", "cap": 1, "tokensUsed": 0, "windowHours": 9000}]}"#,
            r#"{"periodStart": "2026-09-01", "models": [{"model": "A", "cap": 1, "tokensUsed": 0, "creditTokens": -5}]}"#,
        ] {
            let http = Scripted::new().on("GET", CLOUD_QUOTA, 200, body);
            let error = read(&http, json!({ "apiKey": COOKIE })).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::Decoding, "{body}");
            assert_eq!(http.requests().len(), 1, "{body}");
        }
    }

    #[tokio::test]
    async fn an_account_without_models_still_shows_its_balance() {
        let http = Scripted::new()
            .on(
                "GET",
                CLOUD_QUOTA,
                200,
                r#"{"periodStart":"2026-09-01","models":[]}"#,
            )
            .on("GET", CLOUD_BILLING, 200, BASIC_BILLING)
            .on("GET", CLOUD_CREDITS, 200, r#"{"balanceMicros": 0}"#);
        let reading = read(&http, json!({ "apiKey": COOKIE })).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![lines::values(
                "Balance",
                vec![MetricValue::count(0.0, "EUR")]
            )]
        );
    }

    #[test]
    fn the_balance_reads_micros_in_its_currency_and_never_goes_below_zero() {
        assert_eq!(
            balance(&json!({ "balanceMicros": 3_250_000, "currency": "usd" })),
            Some(MetricValue::dollars(3.25))
        );
        assert_eq!(
            balance(&json!({ "balanceMicros": -500_000, "currency": null })),
            Some(MetricValue::count(0.0, "EUR"))
        );
        for credits in [
            json!({ "balanceMicros": 1.5 }),
            json!({ "balanceMicros": "100" }),
            json!({ "balanceMicros": 100, "currency": "euro" }),
            json!({ "balanceMicros": 100, "currency": 978 }),
            json!({ "currency": "EUR" }),
        ] {
            assert_eq!(balance(&credits), None, "{credits}");
        }
    }

    #[tokio::test]
    async fn an_unknown_dashboard_or_a_missing_header_sends_nothing() {
        for secret in [
            json!({ "apiKey": COOKIE, "tenant": "example.com" }),
            json!({ "tenant": "helmcode.com" }),
        ] {
            let http = Scripted::new();
            let error = read(&http, secret.clone()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthInvalid, "{secret}");
            assert!(http.requests().is_empty());
        }
    }

    #[test]
    fn the_dashboard_field_takes_what_a_user_types() {
        for (field, dashboard) in [
            (None, Some(Dashboard::Cloud)),
            (Some(""), Some(Dashboard::Cloud)),
            (Some("helmcode.com"), Some(Dashboard::Cloud)),
            (Some("Helmcode Cloud"), Some(Dashboard::Cloud)),
            (
                Some("https://cloud.helmcode.com/dashboard"),
                Some(Dashboard::Cloud),
            ),
            (Some("cloud-api.helmcode.com"), Some(Dashboard::Cloud)),
            (Some("NaN Builders"), Some(Dashboard::NanBuilders)),
            (Some("nanBuilders"), Some(Dashboard::NanBuilders)),
            (Some(" cloud.nan.builders "), Some(Dashboard::NanBuilders)),
            (Some("helmcode.com.evil.test"), None),
            (Some("nan.builders.example"), None),
        ] {
            assert_eq!(Dashboard::from_field(field), dashboard, "{field:?}");
        }
    }
}
