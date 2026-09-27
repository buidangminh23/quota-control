//! xAI: the prepaid credit and this month's spend of an xAI API team, read with a Management key
//! (xAI Console, Settings > Management Keys); an inference API key is refused.
//!
//! Nothing is read from disk on Windows, macOS or Linux. The key is one saved in Quota Control with
//! the team's ID beside it, or `XAI_MANAGEMENT_API_KEY`; a key without a saved team ID uses
//! `XAI_TEAM_ID`. Each refresh sends the key as a bearer token to
//! `https://management-api.x.ai/v1/billing/teams/{team}`:
//!
//! - `GET …/prepaid/balance`: the team's prepaid ledger. `total.val` is USD cents with credit
//!   negative (a $10 top-up reads `"-1000"`), and zeros may be left out (proto3 JSON). The ledger
//!   takes spend per billing month, so mid-month the console's live credit can be lower than this
//!   by the spend xAI has not posted yet.
//! - `POST …/usage`: the USD spend of each UTC day since the 1st of the month, summed.
//!
//! The balance leads: a refused key, rate limiting or a server error stops the refresh. A team
//! whose balance xAI does not show (400, 403 or 404) still shows its spend, and a key that cannot
//! read usage still shows its balance, each with a notice.

use async_trait::async_trait;
use chrono::{DateTime, Datelike, NaiveDateTime, NaiveTime, Utc};
use serde_json::{Value, json};
use uc_core::{
    HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind, Provider, ProviderLink,
    SimpleProviderError, WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Secret, Service};
use crate::support::{http, lines, value};

pub(crate) struct Xai;

const NAME: &str = "xAI";
const TEAMS: &str = "https://management-api.x.ai/v1/billing/teams";
/// The field the Accounts screen saves the team's ID in, beside the key.
const TEAM_FIELD: &str = "teamId";
/// The team ID of a key that has none saved beside it, such as `XAI_MANAGEMENT_API_KEY`.
const TEAM_ENV: &str = "XAI_TEAM_ID";
/// The analytics time format, as xAI's own example writes it (with the zone `Etc/GMT`).
const TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

const BALANCE: &str = "Balance";
const THIS_MONTH: &str = "Spend This Month";
/// The Balance row's hover note: why it can sit above the console's live credit mid-month.
const POSTED_NOTE: &str =
    "xAI deducts this month's spend from this balance after the month closes.";

const MISSING_KEY: &str = "The xAI Management key is missing. Add it again in Accounts.";
const MISSING_TEAM: &str = "The xAI Team ID is missing. Add the key again in Accounts with its Team ID, or set XAI_TEAM_ID.";
const BAD_TEAM: &str =
    "The xAI Team ID is not valid. Copy it from the team settings in the xAI Console.";
const REFUSED_KEY: &str = "xAI refused this key. Use a Management key from the xAI Console (Settings > Management Keys), not an API key.";
const NO_TEAM: &str = "xAI found no team with this ID. Check the Team ID saved with the key.";
const REFUSED_TEAM: &str =
    "xAI refused this key for this team. Use a Management key of this team with billing access.";
const NO_PREPAID: &str = "xAI shows no prepaid credit for this team.";
const BALANCE_REFUSED: &str = "This key cannot read the team's prepaid credit.";
const USAGE_REFUSED: &str =
    "This key cannot read the team's usage, so Spend This Month is missing.";
const PARTIAL: &str = "xAI returned only part of this month's usage.";

#[async_trait]
impl Service for Xai {
    fn id(&self) -> &'static str {
        "xai"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Status", "https://status.x.ai"),
            ProviderLink::new("Usage", "https://console.x.ai/team/default/usage"),
            ProviderLink::new("Credits", "https://console.x.ai/team/default/billing"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["XAI_MANAGEMENT_API_KEY"],
            url: "https://console.x.ai/team/default/management-keys",
            fields: &[(TEAM_FIELD, "Team ID")],
        })
    }

    /// xAI's own name for the key its billing API takes; an inference API key is refused.
    fn key_label(&self) -> &'static str {
        "Management key"
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let id = |suffix: &str| format!("{}.{suffix}", provider.id);
        let mut balance =
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
                );
        balance.template.info_note = Some(POSTED_NOTE.into());
        vec![
            balance,
            WidgetDescriptor::values(
                id("month"),
                provider,
                THIS_MONTH,
                None,
                Some(MetricKind::Dollars),
                None,
                true,
                None,
                false,
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid(MISSING_KEY))?;
        let team = team_id(context.secret, || environment(TEAM_ENV))?;
        let base = format!("{TEAMS}/{team}");
        let balance = match ask(context, balance_request(&base, key))
            .await
            .read(ledger_balance)
        {
            Answer::Refused(401) => return Err(http::invalid(REFUSED_KEY)),
            Answer::Failed(error) => return Err(error),
            answer => answer,
        };
        let spend = ask(context, usage_request(&base, key, context.now))
            .await
            .read(month_spend);
        reading(balance, spend)
    }
}

/// What one billing endpoint answered.
enum Answer<T> {
    Data(T),
    /// 400, 401, 403 or 404: xAI would not show this team's data to this key.
    Refused(u16),
    /// Any other failure, as the card error it stands for.
    Failed(SimpleProviderError),
}

impl Answer<Value> {
    /// A successful answer read by `read`; one it cannot read is a decoding failure.
    fn read<T>(self, read: impl FnOnce(&Value) -> Option<T>) -> Answer<T> {
        match self {
            Answer::Data(body) => match read(&body) {
                Some(data) => Answer::Data(data),
                None => Answer::Failed(http::decoding(NAME)),
            },
            Answer::Refused(status) => Answer::Refused(status),
            Answer::Failed(error) => Answer::Failed(error),
        }
    }
}

async fn ask(context: &FetchContext<'_>, request: HttpRequest) -> Answer<Value> {
    let response = match http::send(context.http, request, NAME).await {
        Ok(response) => response,
        Err(error) => return Answer::Failed(error),
    };
    if matches!(response.status, 400 | 401 | 403 | 404) {
        return Answer::Refused(response.status);
    }
    if !response.is_success() {
        return Answer::Failed(http::status_error(&response, NAME));
    }
    match http::parse(&response, NAME) {
        Ok(body) => Answer::Data(body),
        Err(error) => Answer::Failed(error),
    }
}

fn balance_request(base: &str, key: &str) -> HttpRequest {
    HttpRequest::get(format!("{base}/prepaid/balance"))
        .bearer(key)
        .header("Accept", "application/json")
}

fn usage_request(base: &str, key: &str, now: DateTime<Utc>) -> HttpRequest {
    HttpRequest::post(format!("{base}/usage"))
        .bearer(key)
        .header("Accept", "application/json")
        .json_body(&usage_query(now))
}

/// The USD spend of each UTC day from the 1st of this month to `now`, in one series.
fn usage_query(now: DateTime<Utc>) -> Value {
    json!({
        "analyticsRequest": {
            "timeRange": {
                "startTime": month_start(now).format(TIME_FORMAT).to_string(),
                "endTime": now.format(TIME_FORMAT).to_string(),
                "timezone": "Etc/GMT"
            },
            "timeUnit": "TIME_UNIT_DAY",
            "values": [{ "name": "usd", "aggregation": "AGGREGATION_SUM" }],
            "groupBy": [],
            "filters": []
        }
    })
}

fn month_start(now: DateTime<Utc>) -> NaiveDateTime {
    let today = now.date_naive();
    today.with_day(1).unwrap_or(today).and_time(NaiveTime::MIN)
}

/// The team whose billing is read: the ID saved beside the key, else `XAI_TEAM_ID`. It becomes a
/// path segment, so only URL-safe IDs pass (xAI's are UUIDs).
fn team_id(
    secret: &Secret,
    environment: impl FnOnce() -> Option<String>,
) -> Result<String, SimpleProviderError> {
    let team = match secret.str(&format!("/{TEAM_FIELD}")) {
        Some(team) => team.to_string(),
        None => environment().ok_or_else(|| http::invalid(MISSING_TEAM))?,
    };
    let url_safe = team.len() <= 128
        && !matches!(team.as_str(), "." | "..")
        && team
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'));
    if url_safe {
        Ok(team)
    } else {
        Err(http::invalid(BAD_TEAM))
    }
}

/// A non-empty environment variable of this process.
fn environment(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// The prepaid credit left, in dollars, from the ledger total in USD cents (credit is negative).
/// proto3 JSON leaves zeros out, so an empty `total`, or a team without any ledger entry, is a
/// real zero; spend beyond the credit reads as nothing left. `None` when no total can be read.
fn ledger_balance(body: &Value) -> Option<f64> {
    let cents = match body.get("total").filter(|total| !total.is_null()) {
        Some(Value::Object(total)) => match total.get("val").filter(|val| !val.is_null()) {
            Some(val) => value::as_number(val)?,
            None => 0.0,
        },
        Some(bare) => value::as_number(bare)?,
        None => {
            let no_ledger = body.is_object()
                && body.get("changes").is_none_or(|changes| {
                    changes.is_null() || changes.as_array().is_some_and(Vec::is_empty)
                });
            if !no_ledger {
                return None;
            }
            0.0
        }
    };
    Some(positive(-cents / 100.0))
}

/// This month's spend in dollars, and whether xAI cut its answer short.
struct Spend {
    dollars: f64,
    partial: bool,
}

/// The month's spend summed over every series and day. proto3 JSON leaves empty lists out, so a
/// month without spend may come without `timeSeries` or `dataPoints`; a day without a number
/// makes the answer unreadable.
fn month_spend(body: &Value) -> Option<Spend> {
    if !body.is_object() {
        return None;
    }
    let mut total = 0.0;
    for series in list(body, "timeSeries")? {
        if !series.is_object() {
            return None;
        }
        for point in list(series, "dataPoints")? {
            total += value::number(point, "/values/0")?;
        }
    }
    Some(Spend {
        dollars: positive(total),
        partial: value::flag(body, "/limitReached") == Some(true),
    })
}

/// The array at `key`: missing or null is an empty list, anything but an array is `None`.
fn list<'a>(object: &'a Value, key: &str) -> Option<&'a [Value]> {
    match object.get(key) {
        None | Some(Value::Null) => Some(&[]),
        Some(Value::Array(items)) => Some(items),
        Some(_) => None,
    }
}

/// `amount` when it is above zero, else a plain zero (never `-0`).
fn positive(amount: f64) -> f64 {
    if amount > 0.0 { amount } else { 0.0 }
}

/// The card from both answers. Each row stands alone: a team whose balance xAI hides still shows
/// its spend, and a key that cannot read usage still shows its balance, each with a notice saying
/// why the other row is missing. When neither row can be read the balance's refusal explains it.
fn reading(balance: Answer<f64>, spend: Answer<Spend>) -> Result<Reading, SimpleProviderError> {
    let mut rows = Vec::new();
    let mut notices = Vec::new();
    match balance {
        Answer::Data(dollars) => {
            rows.push(lines::dollar_value(BALANCE, dollars));
            if matches!(spend, Answer::Refused(401 | 403)) {
                notices.push(USAGE_REFUSED);
            }
        }
        Answer::Refused(status) => match &spend {
            Answer::Data(_) => notices.push(if status == 403 {
                BALANCE_REFUSED
            } else {
                NO_PREPAID
            }),
            Answer::Refused(_) => {
                return Err(http::invalid(if status == 403 {
                    REFUSED_TEAM
                } else {
                    NO_TEAM
                }));
            }
            Answer::Failed(error) => return Err(error.clone()),
        },
        Answer::Failed(error) => return Err(error),
    }
    if let Answer::Data(spend) = spend {
        rows.push(lines::dollar_value(THIS_MONTH, spend.dollars));
        if spend.partial {
            notices.push(PARTIAL);
        }
    }
    let warning = (!notices.is_empty()).then(|| notices.join(" "));
    Ok(Reading::new(None, rows).with_warning(warning))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Roots;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;
    use uc_core::{ErrorCategory, MetricLine};

    const KEY: &str = "xai-mgmt-0123456789abcdef";
    const TEAM: &str = "4f3c2a10-8e6b-4c1d-9a7e-2b5d6c8e9f01";
    const BALANCE_URL: &str = "https://management-api.x.ai/v1/billing/teams/4f3c2a10-8e6b-4c1d-9a7e-2b5d6c8e9f01/prepaid/balance";
    const USAGE_URL: &str =
        "https://management-api.x.ai/v1/billing/teams/4f3c2a10-8e6b-4c1d-9a7e-2b5d6c8e9f01/usage";

    /// A $25 top-up and August's $1.83 spend posted on 10 September: $23.17 of credit left.
    const LEDGER: &str = r#"{
        "changes": [
            {"teamId":"4f3c2a10-8e6b-4c1d-9a7e-2b5d6c8e9f01","changeOrigin":"PURCHASE",
             "topupStatus":"SUCCEEDED","amount":{"val":"-2500"},"invoiceId":"in_0001",
             "invoiceNumber":"XAI-0001","createTime":"2026-08-23T12:56:21Z",
             "createTs":"2026-08-23T12:56:21Z","paymentProcessor":{"kind":"STRIPE"}},
            {"teamId":"4f3c2a10-8e6b-4c1d-9a7e-2b5d6c8e9f01","changeOrigin":"SPEND",
             "amount":{"val":"183"},"spendBpKeyYear":2026,"spendBpKeyMonth":8,
             "createTime":"2026-09-10T21:40:00Z","createTs":"2026-09-10T21:40:00Z"}
        ],
        "total": {"val": "-2317"}
    }"#;

    /// September's daily spend in two series: $1.50 in all.
    const USAGE: &str = r#"{
        "timeSeries": [
            {"group":[],"groupLabels":[],"dataPoints":[
                {"timestamp":"2026-09-25T00:00:00Z","values":[0.75]},
                {"timestamp":"2026-09-26T00:00:00Z","values":[0.5]},
                {"timestamp":"2026-09-27T00:00:00Z","values":[0]}
            ]},
            {"dataPoints":[{"timestamp":"2026-09-26T00:00:00Z","values":[0.25]}]}
        ],
        "limitReached": false
    }"#;

    /// Sunday 27 September 2026, 10:00 UTC.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn scripted(balance: (u16, &str), usage: (u16, &str)) -> Scripted {
        Scripted::new()
            .on("GET", BALANCE_URL, balance.0, balance.1)
            .on("POST", USAGE_URL, usage.0, usage.1)
    }

    async fn fetch_with(http: &Scripted, secret: Value) -> Result<Reading, SimpleProviderError> {
        let scope = context_at(http, secret, now());
        Xai.fetch(&scope.context()).await
    }

    async fn fetch(http: &Scripted) -> Result<Reading, SimpleProviderError> {
        fetch_with(http, json!({ "apiKey": KEY, "teamId": TEAM })).await
    }

    fn urls(http: &Scripted) -> Vec<String> {
        http.requests()
            .into_iter()
            .map(|request| request.url)
            .collect()
    }

    fn balance_row(dollars: f64) -> MetricLine {
        lines::dollar_value("Balance", dollars)
    }

    fn spend_row(dollars: f64) -> MetricLine {
        lines::dollar_value("Spend This Month", dollars)
    }

    #[tokio::test]
    async fn reads_the_prepaid_credit_and_this_months_spend() {
        let http = scripted((200, LEDGER), (200, USAGE));
        let reading = fetch(&http).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(None, vec![balance_row(23.17), spend_row(1.5)])
        );
        assert_eq!(reading.plan, None);
        assert_eq!(reading.warning, None);
    }

    #[tokio::test]
    async fn sends_the_key_as_a_bearer_token_and_asks_for_this_months_daily_spend() {
        let http = scripted((200, LEDGER), (200, USAGE));
        fetch(&http).await.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        for request in &requests {
            assert_eq!(
                header(request, "Authorization"),
                Some("Bearer xai-mgmt-0123456789abcdef")
            );
            assert_eq!(header(request, "Accept"), Some("application/json"));
            assert_eq!(request.timeout, HttpRequest::DEFAULT_TIMEOUT);
        }
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].url, BALANCE_URL);
        assert_eq!(requests[0].body, None);
        assert_eq!(requests[1].method, "POST");
        assert_eq!(requests[1].url, USAGE_URL);
        assert_eq!(
            header(&requests[1], "Content-Type"),
            Some("application/json")
        );
        let body: Value = serde_json::from_slice(requests[1].body.as_ref().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({
                "analyticsRequest": {
                    "timeRange": {
                        "startTime": "2026-09-01 00:00:00",
                        "endTime": "2026-09-27 10:00:00",
                        "timezone": "Etc/GMT"
                    },
                    "timeUnit": "TIME_UNIT_DAY",
                    "values": [{"name": "usd", "aggregation": "AGGREGATION_SUM"}],
                    "groupBy": [],
                    "filters": []
                }
            })
        );
    }

    #[tokio::test]
    async fn a_refused_key_asks_for_nothing_else_and_names_the_management_key() {
        let http = scripted(
            (401, r#"{"code":16,"message":"invalid management key"}"#),
            (200, USAGE),
        );
        let error = fetch(&http).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(
            error.message,
            "xAI refused this key. Use a Management key from the xAI Console (Settings > Management Keys), not an API key."
        );
        assert!(!error.message.contains(KEY));
        assert_eq!(urls(&http), [BALANCE_URL]);
    }

    #[tokio::test]
    async fn rate_limits_and_server_errors_stop_the_refresh_at_once() {
        let http = scripted((429, r#"{"code":8,"message":"too many"}"#), (200, USAGE));
        let error = fetch(&http).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
        assert_eq!(
            error.message,
            "xAI is rate limiting usage requests. Waiting before retrying."
        );
        assert_eq!(urls(&http), [BALANCE_URL]);

        let http = scripted((503, "unavailable"), (200, USAGE));
        let error = fetch(&http).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Http5xx);
        assert_eq!(error.message, "xAI answered with HTTP 503.");
        assert_eq!(urls(&http), [BALANCE_URL]);
    }

    #[tokio::test]
    async fn an_unreadable_balance_is_an_error_never_a_zero() {
        for body in [r#"{"total":{"val":"n/a"}}"#, "<html>busy</html>"] {
            let http = scripted((200, body), (200, USAGE));
            let error = fetch(&http).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::Decoding, "{body}");
            assert_eq!(
                error.message,
                "xAI returned usage data this version cannot read."
            );
            assert_eq!(urls(&http), [BALANCE_URL]);
        }
    }

    #[test]
    fn the_ledger_total_is_negated_cents_and_spend_beyond_the_credit_is_nothing_left() {
        let read = |body: Value| ledger_balance(&body);
        assert_eq!(read(json!({"total": {"val": "-1000"}})), Some(10.0));
        assert_eq!(read(json!({"total": {"val": "-333"}})), Some(3.33));
        assert_eq!(read(json!({"total": {"val": -50}})), Some(0.5));
        assert_eq!(read(json!({"total": "-120"})), Some(1.2));
        let overspent = read(json!({"total": {"val": "2500"}})).unwrap();
        assert_eq!(overspent, 0.0);
        assert!(overspent.is_sign_positive());
        for zero in [
            json!({"total": {"val": "0"}}),
            json!({"total": {}}),
            json!({"total": {"val": null}}),
            json!({}),
            json!({"changes": []}),
            json!({"total": null, "changes": null}),
        ] {
            let balance = read(zero.clone()).unwrap();
            assert_eq!(balance, 0.0, "{zero}");
            assert!(balance.is_sign_positive(), "{zero}");
        }
        for unreadable in [
            json!({"total": {"val": "n/a"}}),
            json!({"total": {"val": true}}),
            json!({"changes": [{"changeOrigin": "PURCHASE", "amount": {"val": "-1000"}}]}),
            json!({"changes": "none"}),
            json!([]),
        ] {
            assert_eq!(read(unreadable.clone()), None, "{unreadable}");
        }
    }

    #[test]
    fn this_months_spend_sums_every_series_and_reads_empty_answers_as_zero() {
        let read = |body: Value| month_spend(&body).map(|spend| (spend.dollars, spend.partial));
        assert_eq!(
            read(serde_json::from_str(USAGE).unwrap()),
            Some((1.5, false))
        );
        assert_eq!(
            read(json!({"timeSeries": [{"dataPoints": [{"values": ["0.25"]}, {"values": [1]}]}]})),
            Some((1.25, false))
        );
        for empty in [
            json!({}),
            json!({"timeSeries": null, "limitReached": false}),
            json!({"timeSeries": []}),
            json!({"timeSeries": [{}]}),
            json!({"timeSeries": [{"dataPoints": null}]}),
        ] {
            assert_eq!(read(empty.clone()), Some((0.0, false)), "{empty}");
        }
        assert_eq!(
            read(json!({"timeSeries": [], "limitReached": true})),
            Some((0.0, true))
        );
        let refunded = month_spend(&json!({"timeSeries": [{"dataPoints": [{"values": [-2]}]}]}))
            .unwrap()
            .dollars;
        assert_eq!(refunded, 0.0);
        assert!(refunded.is_sign_positive());
        for unreadable in [
            json!({"timeSeries": [{"dataPoints": [{"timestamp": "2026-09-27T00:00:00Z"}]}]}),
            json!({"timeSeries": [{"dataPoints": [{"values": []}]}]}),
            json!({"timeSeries": [{"dataPoints": [{"values": ["lots"]}]}]}),
            json!({"timeSeries": [{"dataPoints": {}}]}),
            json!({"timeSeries": [1]}),
            json!({"timeSeries": "none"}),
            json!([]),
        ] {
            assert_eq!(read(unreadable.clone()), None, "{unreadable}");
        }
    }

    #[tokio::test]
    async fn a_capped_usage_answer_adds_a_notice() {
        let capped = USAGE.replace(r#""limitReached": false"#, r#""limitReached": true"#);
        let http = scripted((200, LEDGER), (200, &capped));
        let reading = fetch(&http).await.unwrap();
        assert_eq!(reading.lines, vec![balance_row(23.17), spend_row(1.5)]);
        assert_eq!(
            reading.warning.as_deref(),
            Some("xAI returned only part of this month's usage.")
        );
    }

    #[tokio::test]
    async fn a_key_that_cannot_read_usage_still_shows_the_balance_with_a_notice() {
        for status in [401, 403] {
            let http = scripted((200, LEDGER), (status, "{}"));
            let reading = fetch(&http).await.unwrap();
            assert_eq!(reading.lines, vec![balance_row(23.17)]);
            assert_eq!(
                reading.warning.as_deref(),
                Some("This key cannot read the team's usage, so Spend This Month is missing.")
            );
        }
    }

    #[tokio::test]
    async fn a_failing_usage_request_leaves_only_the_balance() {
        for (status, body) in [
            (500, "boom"),
            (429, "{}"),
            (400, r#"{"message":"bad request"}"#),
            (
                200,
                r#"{"timeSeries":[{"dataPoints":[{"timestamp":"2026-09-27T00:00:00Z"}]}]}"#,
            ),
        ] {
            let http = scripted((200, LEDGER), (status, body));
            let reading = fetch(&http).await.unwrap();
            assert_eq!(
                reading,
                Reading::new(None, vec![balance_row(23.17)]),
                "{status} {body}"
            );
        }
    }

    #[tokio::test]
    async fn a_team_whose_balance_xai_hides_still_shows_its_spend() {
        for (status, notice) in [
            (404, "xAI shows no prepaid credit for this team."),
            (403, "This key cannot read the team's prepaid credit."),
        ] {
            let http = scripted((status, "{}"), (200, USAGE));
            let reading = fetch(&http).await.unwrap();
            assert_eq!(reading.lines, vec![spend_row(1.5)]);
            assert_eq!(reading.warning.as_deref(), Some(notice));
        }
    }

    #[tokio::test]
    async fn a_team_xai_does_not_show_at_all_is_reported_as_a_team_id_problem() {
        for (balance, usage, message) in [
            (
                404,
                404,
                "xAI found no team with this ID. Check the Team ID saved with the key.",
            ),
            (
                400,
                400,
                "xAI found no team with this ID. Check the Team ID saved with the key.",
            ),
            (
                403,
                403,
                "xAI refused this key for this team. Use a Management key of this team with billing access.",
            ),
        ] {
            let http = scripted((balance, "{}"), (usage, "{}"));
            let error = fetch(&http).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthInvalid);
            assert_eq!(error.message, message);
            assert_eq!(urls(&http), [BALANCE_URL, USAGE_URL]);
        }
        let http = scripted((404, "{}"), (503, "unavailable"));
        let error = fetch(&http).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Http5xx);
    }

    #[tokio::test]
    async fn a_missing_key_or_team_sends_nothing() {
        let http = scripted((200, LEDGER), (200, USAGE));
        let error = fetch_with(&http, json!({ "teamId": TEAM }))
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(
            error.message,
            "The xAI Management key is missing. Add it again in Accounts."
        );
        let error = fetch_with(&http, json!({ "apiKey": KEY, "teamId": "team/../other" }))
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        if environment(TEAM_ENV).is_none() {
            let error = fetch_with(&http, json!({ "apiKey": KEY }))
                .await
                .unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthInvalid);
            assert_eq!(
                error.message,
                "The xAI Team ID is missing. Add the key again in Accounts with its Team ID, or set XAI_TEAM_ID."
            );
        }
        assert!(http.requests().is_empty());
    }

    #[test]
    fn the_team_id_comes_from_the_saved_field_else_the_environment_and_must_be_url_safe() {
        let saved = Secret::new(json!({ "apiKey": KEY, "teamId": format!("  {TEAM}  ") }));
        let bare = Secret::new(json!({ "apiKey": KEY }));
        let unset = || None;
        let set = || Some("env-team".to_string());
        assert_eq!(team_id(&saved, set).unwrap(), TEAM);
        assert_eq!(team_id(&bare, set).unwrap(), "env-team");
        let missing = team_id(&bare, unset).unwrap_err();
        assert_eq!(missing.category, ErrorCategory::AuthInvalid);
        assert_eq!(missing.message, MISSING_TEAM);
        for valid in ["team_1.alpha~beta", "ABC-123"] {
            let secret = Secret::new(json!({ "apiKey": KEY, "teamId": valid }));
            assert_eq!(team_id(&secret, unset).unwrap(), valid);
        }
        let long = "a".repeat(129);
        for invalid in [
            "team/../other",
            "..",
            ".",
            "my team",
            "a?b",
            "a#b",
            "é",
            long.as_str(),
        ] {
            let secret = Secret::new(json!({ "apiKey": KEY, "teamId": invalid }));
            let error = team_id(&secret, unset).unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthInvalid, "{invalid}");
            assert_eq!(
                error.message,
                "The xAI Team ID is not valid. Copy it from the team settings in the xAI Console."
            );
        }
    }

    #[test]
    fn the_month_starts_at_utc_midnight_on_the_first() {
        let new_year = Utc.with_ymd_and_hms(2027, 1, 1, 0, 30, 5).unwrap();
        let range = &usage_query(new_year)["analyticsRequest"]["timeRange"];
        assert_eq!(range["startTime"], "2027-01-01 00:00:00");
        assert_eq!(range["endTime"], "2027-01-01 00:30:05");
        let leap = Utc.with_ymd_and_hms(2028, 2, 29, 23, 59, 59).unwrap();
        assert_eq!(
            usage_query(leap)["analyticsRequest"]["timeRange"]["startTime"],
            "2028-02-01 00:00:00"
        );
    }

    #[test]
    fn connects_with_a_management_key_and_its_team_id() {
        let connection = Xai.connection();
        assert_eq!(connection.login_from, None);
        let help = connection.api_key.expect("an API key connection");
        assert_eq!(help.env, ["XAI_MANAGEMENT_API_KEY"]);
        assert_eq!(
            help.url,
            "https://console.x.ai/team/default/management-keys"
        );
        assert_eq!(help.fields, [("teamId", "Team ID")]);
        assert_eq!(Xai.key_label(), "Management key");
        let dir = tempfile::tempdir().unwrap();
        assert!(Xai.discover(&Roots::under(dir.path())).is_empty());
        let links: Vec<_> = Xai
            .links()
            .into_iter()
            .map(|link| (link.label, link.url))
            .collect();
        assert_eq!(
            links,
            [
                ("Status".to_string(), "https://status.x.ai".to_string()),
                (
                    "Usage".to_string(),
                    "https://console.x.ai/team/default/usage".to_string()
                ),
                (
                    "Credits".to_string(),
                    "https://console.x.ai/team/default/billing".to_string()
                ),
            ]
        );
    }

    #[tokio::test]
    async fn every_row_feeds_a_widget() {
        let provider = Provider::new("xai@abc", "xAI");
        let descriptors = Xai.descriptors(&provider);
        let ids: Vec<_> = descriptors.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["xai@abc.balance", "xai@abc.month"]);
        let labels: Vec<_> = descriptors
            .iter()
            .map(|d| d.metric_label.as_str())
            .collect();
        assert_eq!(labels, ["Balance", "Spend This Month"]);
        let balance = &descriptors[0];
        assert_eq!(balance.template.limit, None);
        assert_eq!(
            balance.template.unbounded_value_word.as_deref(),
            Some("left")
        );
        assert_eq!(
            balance.template.info_note.as_deref(),
            Some("xAI deducts this month's spend from this balance after the month closes.")
        );
        assert_eq!(balance.limit_resources.len(), 1);
        let export = &balance.limit_resources[0];
        assert_eq!(export.key, "balance");
        assert_eq!(export.kind, LimitResourceKind::Balance);
        assert_eq!(export.unit, "usd");
        assert_eq!(
            export.source,
            LimitResourceSource::Value {
                kind: MetricKind::Dollars,
                label: None
            }
        );
        let month = &descriptors[1];
        assert_eq!(month.template.selection_kind, Some(MetricKind::Dollars));
        assert!(month.template.is_usage_period);
        assert!(month.limit_resources.is_empty());

        let http = scripted((200, LEDGER), (200, USAGE));
        let reading = fetch(&http).await.unwrap();
        let fed: Vec<_> = reading.lines.iter().map(MetricLine::label).collect();
        assert_eq!(fed, labels);
    }
}
