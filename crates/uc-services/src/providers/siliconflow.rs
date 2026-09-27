//! SiliconFlow: the prepaid balance of a SiliconFlow API key.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `SILICONFLOW_API_KEY` environment variable or from a key saved in Quota Control. A refresh sends
//! `GET /v1/user/info` with the key as a bearer token to `https://api.siliconflow.com`, the global
//! site, whose accounts are billed in US dollars. Keys are issued per site, so a key that host
//! refuses (HTTP 401 or 403) is tried on `https://api.siliconflow.cn`, the mainland-China site,
//! billed in yuan. The host that took the key is remembered for 12 hours after each reading, so
//! later refreshes send one request; a remembered host that refuses the key is forgotten.
//!
//! The answer's `data` carries three decimal strings: `totalBalance` (what the account can spend),
//! `chargeBalance` (money topped up) and `balance` (bonus credit, which SiliconFlow turned into
//! coupons in March 2026). They show as Balance, Paid Balance and Bonus Balance: dollars on the
//! global site, a `CNY` amount on the China site. A negative total means the account is overdue,
//! which pauses model calls and coupons unless it has a credit line; the rows then read zero and
//! the card warns. SiliconFlow is retiring this endpoint (the China site's API reference no longer
//! lists it, and clients have reported HTTP 410 since August 2026), so a 410, or balances left
//! blank the way it blanked the profile fields, reads as a balance that is no longer reported.

use async_trait::async_trait;
use chrono::Duration;
use serde_json::Value;
use uc_core::{
    HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind, MetricLine, MetricValue,
    Provider, ProviderLink, SimpleProviderError, WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct SiliconFlow;

const NAME: &str = "SiliconFlow";
const KEY_PAGE: &str = "https://cloud.siliconflow.com/account/ak";
const USER_INFO: &str = "/v1/user/info";
/// The memo holding the API host that took the key.
const SITE_MEMO: &str = "siliconflow.site";
const BALANCE: &str = "Balance";
const PAID: &str = "Paid Balance";
const BONUS: &str = "Bonus Balance";
/// Each row's title and its widget id suffix, which is also its limits export key.
const ROWS: [(&str, &str); 3] = [
    (BALANCE, "balance"),
    (PAID, "paidBalance"),
    (BONUS, "bonusBalance"),
];
/// The unit word of a yuan amount, which its limits export also matches.
const CNY: &str = "CNY";
const MISSING_KEY: &str = "The SiliconFlow API key is missing. Add it again in Accounts.";
const REFUSED: &str = "SiliconFlow refused this API key. Check it or create a new one.";

/// The currency a site bills its accounts in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Currency {
    Usd,
    Cny,
}

/// A SiliconFlow site: its API host, the console where its balance can be read, and its currency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Site {
    api: &'static str,
    console: &'static str,
    currency: Currency,
}

/// The global site, tried first, then the mainland-China site a key may belong to instead.
const SITES: [Site; 2] = [
    Site {
        api: "https://api.siliconflow.com",
        console: "cloud.siliconflow.com",
        currency: Currency::Usd,
    },
    Site {
        api: "https://api.siliconflow.cn",
        console: "cloud.siliconflow.cn",
        currency: Currency::Cny,
    },
];

#[async_trait]
impl Service for SiliconFlow {
    fn id(&self) -> &'static str {
        "siliconflow"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Dashboard", "https://cloud.siliconflow.com"),
            ProviderLink::new("API Keys", KEY_PAGE),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["SILICONFLOW_API_KEY"],
            url: KEY_PAGE,
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        ROWS.iter()
            .map(|(title, key)| {
                WidgetDescriptor::dollar_balance(
                    format!("{}.{key}", provider.id),
                    provider,
                    title,
                    None,
                    "left",
                )
                .exporting_limit(
                    key,
                    LimitResourceKind::Balance,
                    "usd",
                    LimitResourceSource::Value {
                        kind: MetricKind::Dollars,
                        label: None,
                    },
                    false,
                )
                .exporting_limit(
                    &format!("{key}Cny"),
                    LimitResourceKind::Balance,
                    "cny",
                    LimitResourceSource::Value {
                        kind: MetricKind::Count,
                        label: Some(CNY.into()),
                    },
                    false,
                )
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid(MISSING_KEY))?;
        let (site, body) = user_info(context, key).await?;
        let data = body
            .get("data")
            .filter(|data| data.is_object())
            .ok_or_else(|| http::decoding(NAME))?;
        let balances = balances(data, site)?;
        let warning = (balances.total < 0.0).then(|| overdue(site.currency, -balances.total));
        Ok(Reading::new(None, balances.rows(site.currency)).with_warning(warning))
    }
}

/// The user info answer and the site whose host gave it: the remembered site, else the global
/// site and, when that one refuses the key, the China site.
async fn user_info(
    context: &FetchContext<'_>,
    key: &str,
) -> Result<(Site, Value), SimpleProviderError> {
    let remembered = context
        .memo
        .get(SITE_MEMO, context.now)
        .await
        .and_then(|api| {
            SITES
                .into_iter()
                .find(|site| api.as_str() == Some(site.api))
        });
    let sites = remembered.map_or(SITES.to_vec(), |site| vec![site]);
    let mut forbidden = None;
    for site in sites {
        let response = http::send(
            context.http,
            HttpRequest::get(format!("{}{USER_INFO}", site.api))
                .bearer(key)
                .header("Accept", "application/json"),
            NAME,
        )
        .await?;
        match response.status {
            // A key this site does not know: it may belong to the other site.
            401 => {}
            // A key whose account this site will not serve; the other site still gets a try.
            403 => {
                if forbidden.is_none() {
                    forbidden = Some(response);
                }
            }
            // The site took the key but no longer serves the balance.
            410 => {
                remember(context, site).await;
                return Err(no_longer_reported(site));
            }
            _ if response.is_success() => {
                let body = http::parse(&response, NAME)?;
                remember(context, site).await;
                return Ok((site, body));
            }
            _ => return Err(http::status_error(&response, NAME)),
        }
    }
    context.memo.remove(SITE_MEMO).await;
    Err(match forbidden {
        Some(response) => http::status_error(&response, NAME),
        None => http::invalid(REFUSED),
    })
}

/// Remembers the site that took the key for the next 12 hours.
async fn remember(context: &FetchContext<'_>, site: Site) {
    context
        .memo
        .put(
            SITE_MEMO,
            Value::String(site.api.to_string()),
            Some(context.now + Duration::hours(12)),
        )
        .await;
}

/// What the account holds: the total it can spend, and its paid and bonus parts when stated.
struct Balances {
    total: f64,
    paid: Option<f64>,
    bonus: Option<f64>,
}

impl Balances {
    /// One row per stated amount, the total first.
    fn rows(&self, currency: Currency) -> Vec<MetricLine> {
        [
            (BALANCE, Some(self.total)),
            (PAID, self.paid),
            (BONUS, self.bonus),
        ]
        .into_iter()
        .filter_map(|(label, amount)| {
            amount.map(|amount| lines::values(label, vec![currency.value(amount)]))
        })
        .collect()
    }
}

/// The balances in `data`. The total is `totalBalance`, or the paid and bonus parts added up when
/// an answer leaves it out.
fn balances(data: &Value, site: Site) -> Result<Balances, SimpleProviderError> {
    let paid = value::number(data, "/chargeBalance");
    let bonus = value::number(data, "/balance");
    if let Some(total) = value::number(data, "/totalBalance").or_else(|| Some(paid? + bonus?)) {
        return Ok(Balances { total, paid, bonus });
    }
    // SiliconFlow retires a field by answering it empty, as it did with the profile fields.
    let blank = |field: &str| {
        data.get(field).is_some_and(|found| {
            found.is_null() || found.as_str().is_some_and(|text| text.trim().is_empty())
        })
    };
    if ["totalBalance", "chargeBalance", "balance"]
        .into_iter()
        .any(blank)
    {
        Err(no_longer_reported(site))
    } else {
        Err(http::decoding(NAME))
    }
}

/// The error for a balance SiliconFlow no longer reports through its API.
fn no_longer_reported(site: Site) -> SimpleProviderError {
    http::not_available(format!(
        "SiliconFlow no longer reports this account's balance through its API. Check it at {}.",
        site.console
    ))
}

/// The notice for an overdue account, which SiliconFlow stops serving, coupons included, unless
/// the account has a credit line.
fn overdue(currency: Currency, owed: f64) -> String {
    format!(
        "The SiliconFlow balance is {} below zero. Unless the account has a credit line, model calls and coupons stay paused until you top up.",
        currency.prose(owed)
    )
}

impl Currency {
    /// `amount` as a row value, never below zero: dollars, or a `CNY` count.
    fn value(self, amount: f64) -> MetricValue {
        let amount = if amount > 0.0 { amount } else { 0.0 };
        match self {
            Currency::Usd => MetricValue::dollars(amount),
            Currency::Cny => MetricValue::count(amount, CNY),
        }
    }

    /// `amount` written in a sentence, rounded up to a whole cent so that a tiny debt still shows:
    /// `$3.20`, `3.20 CNY`.
    fn prose(self, amount: f64) -> String {
        let cents = (amount * 100.0 - 1e-6).ceil().max(1.0) as i64;
        let text = format!("{}.{:02}", cents / 100, cents % 100);
        match self {
            Currency::Usd => format!("${text}"),
            Currency::Cny => format!("{text} {CNY}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Memo, Roots, Secret};
    use crate::testing::{Scripted, context_at, header};
    use chrono::{DateTime, TimeZone, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const KEY: &str = "sk-siliconflow-test-0123456789abcdef";
    const GLOBAL_INFO: &str = "https://api.siliconflow.com/v1/user/info";
    const CHINA_INFO: &str = "https://api.siliconflow.cn/v1/user/info";
    /// How both hosts answer a key they do not know.
    const UNKNOWN_KEY: &str = r#"{"code":30014,"data":null,"message":"Token is invalid."}"#;
    /// A user info answer shaped like SiliconFlow's documentation, profile fields blanked.
    const GLOBAL_ANSWER: &str = r#"{"code":20000,"message":"OK","status":true,"data":{
        "id":"userid","name":"","image":"","email":"","isAdmin":false,
        "balance":"1.25","status":"normal","introduction":"","role":"",
        "chargeBalance":"40.00","totalBalance":"41.25"}}"#;
    const CHINA_ANSWER: &str = r#"{"code":20000,"message":"OK","status":true,"data":{
        "id":"userid","isAdmin":false,"balance":"14.00","status":"normal",
        "chargeBalance":"100.50","totalBalance":"114.50"}}"#;
    const NO_LONGER_REPORTED: &str =
        "SiliconFlow no longer reports this account's balance through its API. Check it at";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn secret() -> Value {
        json!({ "apiKey": KEY })
    }

    fn row(label: &str, value: MetricValue) -> MetricLine {
        lines::values(label, vec![value])
    }

    fn urls(http: &Scripted) -> Vec<String> {
        http.requests()
            .into_iter()
            .map(|request| request.url)
            .collect()
    }

    async fn fetch(http: &Scripted) -> Result<Reading, SimpleProviderError> {
        let scope = context_at(http, secret(), now());
        SiliconFlow.fetch(&scope.context()).await
    }

    #[tokio::test]
    async fn a_global_key_shows_its_three_balances_in_dollars() {
        let http = Scripted::new().on("GET", GLOBAL_INFO, 200, GLOBAL_ANSWER);
        let reading = fetch(&http).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                None,
                vec![
                    row("Balance", MetricValue::dollars(41.25)),
                    row("Paid Balance", MetricValue::dollars(40.0)),
                    row("Bonus Balance", MetricValue::dollars(1.25)),
                ]
            )
        );
        assert_eq!(urls(&http), [GLOBAL_INFO]);
        let provider = Provider::new("siliconflow@abc", NAME);
        let labels: Vec<String> = SiliconFlow
            .descriptors(&provider)
            .into_iter()
            .map(|descriptor| descriptor.metric_label)
            .collect();
        let read: Vec<&str> = reading.lines.iter().map(MetricLine::label).collect();
        assert_eq!(read, labels);
    }

    #[tokio::test]
    async fn a_key_the_global_site_refuses_is_read_from_the_china_site_in_yuan_and_remembered() {
        let http = Scripted::new().on("GET", GLOBAL_INFO, 401, UNKNOWN_KEY).on(
            "GET",
            CHINA_INFO,
            200,
            CHINA_ANSWER,
        );
        let scope = context_at(&http, secret(), now());
        let expected = Reading::new(
            None,
            vec![
                row("Balance", MetricValue::count(114.5, "CNY")),
                row("Paid Balance", MetricValue::count(100.5, "CNY")),
                row("Bonus Balance", MetricValue::count(14.0, "CNY")),
            ],
        );
        assert_eq!(SiliconFlow.fetch(&scope.context()).await.unwrap(), expected);
        assert_eq!(SiliconFlow.fetch(&scope.context()).await.unwrap(), expected);
        assert_eq!(urls(&http), [GLOBAL_INFO, CHINA_INFO, CHINA_INFO]);
        assert_eq!(
            header(&http.requests()[1], "Authorization"),
            Some("Bearer sk-siliconflow-test-0123456789abcdef")
        );
    }

    #[tokio::test]
    async fn the_remembered_site_is_checked_again_after_twelve_hours_without_a_reading() {
        let http = Scripted::new().on("GET", GLOBAL_INFO, 401, UNKNOWN_KEY).on(
            "GET",
            CHINA_INFO,
            200,
            CHINA_ANSWER,
        );
        let shared = http.shared();
        let secret = Secret::new(secret());
        let memo = Memo::default();
        // Every reading remembers the site for another 12 hours from its own time.
        let last = 11 * 60 + 59;
        for minutes in [0, last, last + 12 * 60] {
            let context = FetchContext {
                secret: &secret,
                http: &shared,
                now: now() + Duration::minutes(minutes),
                memo: &memo,
            };
            SiliconFlow.fetch(&context).await.unwrap();
        }
        assert_eq!(
            urls(&http),
            [GLOBAL_INFO, CHINA_INFO, CHINA_INFO, GLOBAL_INFO, CHINA_INFO]
        );
    }

    #[tokio::test]
    async fn a_key_both_sites_refuse_is_reported_as_invalid_without_revealing_it() {
        let http = Scripted::new().on("GET", GLOBAL_INFO, 401, UNKNOWN_KEY).on(
            "GET",
            CHINA_INFO,
            401,
            UNKNOWN_KEY,
        );
        let scope = context_at(&http, secret(), now());
        let error = SiliconFlow.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(
            error.message,
            "SiliconFlow refused this API key. Check it or create a new one."
        );
        assert!(!error.message.contains(KEY));
        SiliconFlow.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(
            urls(&http),
            [GLOBAL_INFO, CHINA_INFO, GLOBAL_INFO, CHINA_INFO]
        );
    }

    #[tokio::test]
    async fn a_remembered_site_that_starts_refusing_the_key_is_forgotten() {
        let http = Scripted::new()
            .on("GET", GLOBAL_INFO, 200, GLOBAL_ANSWER)
            .on("GET", GLOBAL_INFO, 401, UNKNOWN_KEY)
            .on("GET", CHINA_INFO, 401, UNKNOWN_KEY);
        let scope = context_at(&http, secret(), now());
        SiliconFlow.fetch(&scope.context()).await.unwrap();
        let error = SiliconFlow.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        SiliconFlow.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(
            urls(&http),
            [GLOBAL_INFO, GLOBAL_INFO, GLOBAL_INFO, CHINA_INFO]
        );
    }

    #[tokio::test]
    async fn an_account_a_site_forbids_is_reported_as_refused_after_trying_the_other_site() {
        let http = Scripted::new().on("GET", GLOBAL_INFO, 403, "Forbidden").on(
            "GET",
            CHINA_INFO,
            401,
            UNKNOWN_KEY,
        );
        let error = fetch(&http).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(
            error.message,
            "SiliconFlow refused the saved login or key. Sign in again or replace the key."
        );
        assert_eq!(urls(&http), [GLOBAL_INFO, CHINA_INFO]);
    }

    #[tokio::test]
    async fn rate_limits_and_server_errors_are_reported_without_trying_the_china_site() {
        let limited = Scripted::new().on("GET", GLOBAL_INFO, 429, "RateLimit");
        let error = fetch(&limited).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
        assert_eq!(
            error.message,
            "SiliconFlow is rate limiting usage requests. Waiting before retrying."
        );
        assert_eq!(urls(&limited), [GLOBAL_INFO]);
        let overloaded = Scripted::new().on("GET", GLOBAL_INFO, 503, "Overloaded");
        let error = fetch(&overloaded).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Http5xx);
        assert_eq!(error.message, "SiliconFlow answered with HTTP 503.");
        assert_eq!(urls(&overloaded), [GLOBAL_INFO]);
    }

    #[tokio::test]
    async fn a_retired_endpoint_reads_as_not_available_and_points_to_that_sites_console() {
        let http = Scripted::new()
            .on("GET", GLOBAL_INFO, 401, UNKNOWN_KEY)
            .on("GET", CHINA_INFO, 410, "Gone");
        let scope = context_at(&http, secret(), now());
        let error = SiliconFlow.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(
            error.message,
            format!("{NO_LONGER_REPORTED} cloud.siliconflow.cn.")
        );
        SiliconFlow.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(urls(&http), [GLOBAL_INFO, CHINA_INFO, CHINA_INFO]);
    }

    #[tokio::test]
    async fn balances_left_blank_read_as_no_longer_reported() {
        let http = Scripted::new().on(
            "GET",
            GLOBAL_INFO,
            200,
            r#"{"code":20000,"message":"OK","status":true,"data":{"id":"userid","balance":"","chargeBalance":"","totalBalance":"","status":"normal"}}"#,
        );
        let error = fetch(&http).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(
            error.message,
            format!("{NO_LONGER_REPORTED} cloud.siliconflow.com.")
        );
    }

    #[tokio::test]
    async fn an_overdue_account_reads_zero_and_warns_with_what_it_owes() {
        let http = Scripted::new().on(
            "GET",
            GLOBAL_INFO,
            200,
            r#"{"code":20000,"message":"OK","status":true,"data":{"balance":"0.00","chargeBalance":"-3.20","totalBalance":"-3.20","status":"normal"}}"#,
        );
        let reading = fetch(&http).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                None,
                vec![
                    row("Balance", MetricValue::dollars(0.0)),
                    row("Paid Balance", MetricValue::dollars(0.0)),
                    row("Bonus Balance", MetricValue::dollars(0.0)),
                ]
            )
            .with_warning(Some(
                "The SiliconFlow balance is $3.20 below zero. Unless the account has a credit line, model calls and coupons stay paused until you top up."
                    .into()
            ))
        );
    }

    #[tokio::test]
    async fn a_total_left_out_is_the_paid_and_bonus_parts_added_up() {
        let http = Scripted::new().on(
            "GET",
            GLOBAL_INFO,
            200,
            r#"{"code":20000,"message":"OK","status":true,"data":{"balance":"0.50","chargeBalance":5.5}}"#,
        );
        let reading = fetch(&http).await.unwrap();
        assert_eq!(
            reading.lines,
            [
                row("Balance", MetricValue::dollars(6.0)),
                row("Paid Balance", MetricValue::dollars(5.5)),
                row("Bonus Balance", MetricValue::dollars(0.5)),
            ]
        );
    }

    #[tokio::test]
    async fn a_negative_zero_total_is_a_plain_zero_without_a_warning() {
        let http = Scripted::new().on(
            "GET",
            GLOBAL_INFO,
            200,
            r#"{"code":20000,"message":"OK","status":true,"data":{"totalBalance":"-0.00"}}"#,
        );
        let reading = fetch(&http).await.unwrap();
        assert_eq!(reading.warning, None);
        assert_eq!(reading.lines.len(), 1);
        let MetricLine::Values(line) = &reading.lines[0] else {
            panic!("expected a values row")
        };
        assert_eq!(line.label, "Balance");
        assert!(line.values[0].number == 0.0 && line.values[0].number.is_sign_positive());
    }

    #[tokio::test]
    async fn answers_without_readable_balances_are_decoding_errors() {
        for body in [
            r#"{"code":20000,"message":"OK","status":true,"data":{"id":"userid","status":"normal"}}"#,
            r#"{"code":20000,"message":"OK","status":true,"data":null}"#,
            r#"{"code":20000,"message":"OK","status":true,"data":{"totalBalance":"about three dollars"}}"#,
            "<html>busy</html>",
        ] {
            let http = Scripted::new().on("GET", GLOBAL_INFO, 200, body);
            let error = fetch(&http).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::Decoding, "{body}");
            assert_eq!(
                error.message,
                "SiliconFlow returned usage data this version cannot read."
            );
        }
    }

    #[tokio::test]
    async fn sends_the_key_as_a_bearer_token_to_the_user_info_endpoint() {
        let http = Scripted::new().on("GET", GLOBAL_INFO, 200, GLOBAL_ANSWER);
        fetch(&http).await.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, "GET");
        assert_eq!(request.url, "https://api.siliconflow.com/v1/user/info");
        assert_eq!(
            header(request, "Authorization"),
            Some("Bearer sk-siliconflow-test-0123456789abcdef")
        );
        assert_eq!(header(request, "Accept"), Some("application/json"));
        assert_eq!(request.body, None);
        assert_eq!(request.timeout, HttpRequest::DEFAULT_TIMEOUT);
    }

    #[tokio::test]
    async fn a_secret_without_a_key_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), now());
        let error = SiliconFlow.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(
            error.message,
            "The SiliconFlow API key is missing. Add it again in Accounts."
        );
        assert!(http.requests().is_empty());
    }

    #[test]
    fn amounts_in_notices_round_up_to_a_whole_cent() {
        assert_eq!(Currency::Usd.prose(3.2), "$3.20");
        assert_eq!(Currency::Usd.prose(0.001), "$0.01");
        assert_eq!(Currency::Cny.prose(12.345), "12.35 CNY");
        assert_eq!(Currency::Cny.prose(1500.0), "1500.00 CNY");
    }

    #[test]
    fn connects_with_an_api_key_only() {
        assert_eq!(SiliconFlow.id(), "siliconflow");
        assert_eq!(SiliconFlow.name(), "SiliconFlow");
        let connection = SiliconFlow.connection();
        assert_eq!(connection.login_from, None);
        let help = connection.api_key.expect("an API key connection");
        assert_eq!(help.env, ["SILICONFLOW_API_KEY"]);
        assert_eq!(help.url, "https://cloud.siliconflow.com/account/ak");
        assert!(help.fields.is_empty());
        assert_eq!(
            SiliconFlow.links(),
            [
                ProviderLink::new("Dashboard", "https://cloud.siliconflow.com"),
                ProviderLink::new("API Keys", "https://cloud.siliconflow.com/account/ak"),
            ]
        );
        let dir = tempfile::tempdir().unwrap();
        assert!(SiliconFlow.discover(&Roots::under(dir.path())).is_empty());
    }

    #[test]
    fn the_widgets_read_the_balance_rows_and_export_both_currencies() {
        let provider = Provider::new("siliconflow@abc", NAME);
        let descriptors = SiliconFlow.descriptors(&provider);
        let widgets: Vec<_> = descriptors
            .iter()
            .map(|descriptor| {
                (
                    descriptor.id.as_str(),
                    descriptor.metric_label.as_str(),
                    descriptor.template.limit,
                    descriptor.template.unbounded_value_word.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            widgets,
            [
                ("siliconflow@abc.balance", "Balance", None, Some("left")),
                (
                    "siliconflow@abc.paidBalance",
                    "Paid Balance",
                    None,
                    Some("left")
                ),
                (
                    "siliconflow@abc.bonusBalance",
                    "Bonus Balance",
                    None,
                    Some("left")
                ),
            ]
        );
        let usd = LimitResourceSource::Value {
            kind: MetricKind::Dollars,
            label: None,
        };
        let cny = LimitResourceSource::Value {
            kind: MetricKind::Count,
            label: Some("CNY".into()),
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
        let balance = LimitResourceKind::Balance;
        assert_eq!(
            exports,
            [
                ("balance", balance, "usd", &usd, false),
                ("balanceCny", balance, "cny", &cny, false),
                ("paidBalance", balance, "usd", &usd, false),
                ("paidBalanceCny", balance, "cny", &cny, false),
                ("bonusBalance", balance, "usd", &usd, false),
                ("bonusBalanceCny", balance, "cny", &cny, false),
            ]
        );
    }
}
