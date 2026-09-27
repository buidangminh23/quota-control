//! Chutes: the included usage and the pay-as-you-go balance behind a Chutes API key.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the `CHUTES_API_KEY`
//! environment variable or from a key saved in Quota Control. A refresh sends three requests to
//! `https://api.chutes.ai` together, each with the key as a bearer token. Their answers follow
//! Chutes' open-source API server (`chutesai/chutes-api`, `api/user/router.py`):
//! - `GET /users/me/subscription_usage`: a Base, Plus or Pro subscription's 4-hour and monthly
//!   caps, counted in pay-as-you-go dollars (the monthly cap is five times the plan's price, and a
//!   custom subscription has none), with the monthly renewal; `{"subscription": false}` without a
//!   subscription.
//! - `GET /users/me/quota_usage/me`: the requests included per UTC day and those used today. `me`
//!   names no chute, so the answer is the account-wide quota and counter.
//! - `GET /users/me`: the account, for its balance in dollars.
//!
//! Past the included usage Chutes charges requests to the balance. When the balance is empty too,
//! paid models refuse requests, and the card warns about it.

use async_trait::async_trait;
use chrono::{DateTime, Months, TimeZone, Utc};
use serde_json::Value;
use uc_core::{
    ErrorCategory, HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind, MetricLine,
    PlanTerm, Provider, ProviderLink, SimpleProviderError, WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct Chutes;

const NAME: &str = "Chutes";
const SUBSCRIPTION_URL: &str = "https://api.chutes.ai/users/me/subscription_usage";
const DAILY_URL: &str = "https://api.chutes.ai/users/me/quota_usage/me";
const ACCOUNT_URL: &str = "https://api.chutes.ai/users/me";
/// What each request reads, in request order, for the notice about a part that failed.
const PARTS: [&str; 3] = ["subscription usage", "daily quota", "balance"];

const SESSION: &str = "Session";
const DAILY: &str = "Daily";
const MONTHLY: &str = "Monthly";
const BALANCE: &str = "Balance";
/// The Session cap counts in fixed 4-hour UTC blocks (00:00, 04:00, …).
const SESSION_MS: i64 = 4 * lines::HOUR_MS;

/// Chutes' plans by monthly price in dollars.
const PLANS: [(f64, &str); 3] = [(3.0, "Base"), (10.0, "Plus"), (20.0, "Pro")];
const CUSTOM_PLAN: &str = "Custom";
/// A subscription at a price this version does not know.
const OTHER_PLAN: &str = "Subscription";
const PAY_AS_YOU_GO: &str = "Pay As You Go";

const MISSING_KEY: &str = "The Chutes API key is missing. Add it again in Accounts.";
/// Chutes answers 401 alike for an unknown key and for a key whose scope leaves out the account.
const REFUSED: &str = "Chutes refused this API key. Check it, or create a key with account access.";
const BLOCKED: &str =
    "The included usage is used up and the balance is empty, so paid models will refuse requests.";

#[async_trait]
impl Service for Chutes {
    fn id(&self) -> &'static str {
        "chutes"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Status", "https://status.chutes.ai"),
            ProviderLink::new("Usage", "https://chutes.ai/app/settings/usage"),
            ProviderLink::new("Billing", "https://chutes.ai/app/settings/billing"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["CHUTES_API_KEY"],
            url: "https://chutes.ai/app/api",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::percent(
                format!("{}.session", provider.id),
                provider,
                SESSION,
                None,
                None,
            )
            .exporting_progress("session", "percent"),
            // Pro includes 5,000 requests a day; the real limit comes with each reading.
            WidgetDescriptor::bounded_count(
                format!("{}.daily", provider.id),
                provider,
                DAILY,
                None,
                5000.0,
                "requests",
                Some(lines::DAY_MS),
            )
            .exporting_progress("daily", "requests"),
            WidgetDescriptor::percent(
                format!("{}.monthly", provider.id),
                provider,
                MONTHLY,
                None,
                None,
            )
            .exporting_progress("monthly", "percent"),
            WidgetDescriptor::dollar_balance(
                format!("{}.balance", provider.id),
                provider,
                BALANCE,
                None,
                "left",
            )
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
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid(MISSING_KEY))?;
        let (subscription, daily, account) = tokio::join!(
            get(context, key, SUBSCRIPTION_URL),
            get(context, key, DAILY_URL),
            get(context, key, ACCOUNT_URL),
        );
        let subscription = subscription.and_then(|body| read_subscription(&body, context.now));
        let daily = daily.and_then(|body| read_daily(&body));
        let balance = account.and_then(|body| read_balance(&body));
        let errors = [
            subscription.as_ref().err(),
            daily.as_ref().err(),
            balance.as_ref().err(),
        ];
        if let Some(error) = card_error(&errors) {
            return Err(error);
        }
        let missing: Vec<&str> = PARTS
            .into_iter()
            .zip(errors)
            .filter_map(|(part, error)| error.map(|_| part))
            .collect();
        let (subscription, daily, balance) = (subscription.ok(), daily.ok(), balance.ok());
        let warning = if !missing.is_empty() {
            Some(format!(
                "Could not read the Chutes {} this time.",
                missing.join(" and ")
            ))
        } else if let (Some(subscription), Some(daily), Some(balance)) =
            (&subscription, &daily, balance)
            && blocked(subscription, daily, balance)
        {
            Some(BLOCKED.to_string())
        } else {
            None
        };
        // Without a subscription, requests past any included daily quota are paid from the
        // balance; free and invoiced accounts (an unlimited quota) get no plan name.
        let plan = match (&subscription, &daily) {
            (
                Some(Subscription {
                    plan: Some(plan), ..
                }),
                _,
            ) => Some(plan.clone()),
            (Some(_), Some(Daily::Quota { .. })) => Some(PAY_AS_YOU_GO.to_string()),
            _ => None,
        };
        let renews_at = subscription
            .as_ref()
            .and_then(|subscription| subscription.renews_at);
        let (session, monthly) = subscription.map_or((None, None), |subscription| {
            (subscription.session, subscription.monthly)
        });
        let rows: Vec<MetricLine> = [
            session,
            daily.as_ref().and_then(|daily| daily.line(context.now)),
            monthly,
            balance.map(|balance| lines::dollar_value(BALANCE, balance.max(0.0))),
        ]
        .into_iter()
        .flatten()
        .collect();
        Ok(Reading::new(plan, rows)
            .with_plan_term(renews_at.map(|ends_at| PlanTerm::Stated {
                ends_at,
                checked_at: None,
            }))
            .with_warning(warning))
    }
}

/// One account request. A refused key is reported as such rather than as a status.
async fn get(
    context: &FetchContext<'_>,
    key: &str,
    url: &str,
) -> Result<Value, SimpleProviderError> {
    let response = http::send(
        context.http,
        HttpRequest::get(url)
            .bearer(key)
            .header("Accept", "application/json"),
        NAME,
    )
    .await?;
    if matches!(response.status, 401 | 403) {
        return Err(http::invalid(REFUSED));
    }
    if !response.is_success() {
        return Err(http::status_error(&response, NAME));
    }
    http::parse(&response, NAME)
}

/// The error that fails the whole card: a refused key, then a rate limit (so refreshing pauses),
/// then, when nothing could be read, the first failure. Any other failure leaves out only the rows
/// of the part that failed.
fn card_error(errors: &[Option<&SimpleProviderError>]) -> Option<SimpleProviderError> {
    let failed: Vec<&SimpleProviderError> = errors.iter().flatten().copied().collect();
    failed
        .iter()
        .find(|error| error.category == ErrorCategory::AuthInvalid)
        .or_else(|| {
            failed
                .iter()
                .find(|error| error.category == ErrorCategory::RateLimited)
        })
        .or_else(|| {
            (failed.len() == errors.len())
                .then(|| failed.first())
                .flatten()
        })
        .map(|error| (*error).clone())
}

/// What `subscription_usage` says; all empty without a subscription.
#[derive(Default)]
struct Subscription {
    plan: Option<String>,
    session: Option<MetricLine>,
    monthly: Option<MetricLine>,
    /// When the subscription renews, which is also when the monthly cap starts over.
    renews_at: Option<DateTime<Utc>>,
    /// Whether a cap is used up, so Chutes charges requests to the balance.
    capped: bool,
}

/// A cap's usage and size in pay-as-you-go dollars, and when it starts over.
struct Cap {
    usage: f64,
    size: f64,
    resets_at: Option<DateTime<Utc>>,
}

impl Cap {
    fn line(&self, label: &str, period_ms: i64) -> Option<MetricLine> {
        lines::percent_of(
            label,
            self.usage,
            self.size,
            self.resets_at,
            Some(period_ms),
        )
    }

    /// Chutes' own test for a reached cap.
    fn used_up(&self) -> bool {
        self.usage >= self.size
    }
}

/// A `subscription_usage` answer: the plan, the Session (4-hour) and Monthly caps as used shares,
/// and the renewal. A custom subscription states no monthly cap, only the anchor it renews from a
/// month later, which is how Chutes computes every renewal.
fn read_subscription(
    body: &Value,
    now: DateTime<Utc>,
) -> Result<Subscription, SimpleProviderError> {
    match value::flag(body, "/subscription") {
        Some(false) => return Ok(Subscription::default()),
        Some(true) => {}
        None => return Err(http::decoding(NAME)),
    }
    let session = read_cap(body, "/four_hour", now)?;
    let monthly = if value::flag(body, "/monthly/uncapped") == Some(true) {
        None
    } else {
        Some(read_cap(body, "/monthly", now)?)
    };
    let renews_at = match &monthly {
        Some(monthly) => monthly.resets_at,
        None => value::time(body, "/anchor_date")
            .and_then(|anchor| anchor.checked_add_months(Months::new(1)))
            .filter(|renewal| *renewal > now),
    };
    let plan = if value::flag(body, "/custom") == Some(true) {
        CUSTOM_PLAN
    } else {
        value::number(body, "/monthly_price")
            .and_then(|price| {
                PLANS
                    .iter()
                    .find(|(tier_price, _)| (tier_price - price).abs() < 0.01)
            })
            .map_or(OTHER_PLAN, |(_, name)| *name)
    };
    Ok(Subscription {
        plan: Some(plan.to_string()),
        session: session.line(SESSION, SESSION_MS),
        monthly: monthly
            .as_ref()
            .and_then(|monthly| monthly.line(MONTHLY, lines::MONTH_MS)),
        renews_at,
        capped: session.used_up() || monthly.as_ref().is_some_and(Cap::used_up),
    })
}

fn read_cap(body: &Value, pointer: &str, now: DateTime<Utc>) -> Result<Cap, SimpleProviderError> {
    let cap = body.pointer(pointer).ok_or_else(|| http::decoding(NAME))?;
    match (value::number(cap, "/usage"), value::number(cap, "/cap")) {
        (Some(usage), Some(size)) => Ok(Cap {
            usage,
            size,
            // A reset that already passed would read as due; the next refresh brings the new one.
            resets_at: value::time(cap, "/reset_at").filter(|reset| *reset > now),
        }),
        _ => Err(http::decoding(NAME)),
    }
}

/// The account's daily request quota.
enum Daily {
    /// Free and invoiced accounts have no daily limit.
    Unlimited,
    /// The requests included per UTC day, and those used today.
    Quota { used: f64, quota: f64 },
}

impl Daily {
    /// The Daily row, which starts over at the next 00:00 UTC. An account without included
    /// requests (pay-as-you-go) has none.
    fn line(&self, now: DateTime<Utc>) -> Option<MetricLine> {
        match *self {
            Daily::Quota { used, quota } if quota > 0.0 => Some(lines::count(
                DAILY,
                used,
                quota,
                "requests",
                next_utc_midnight(now),
                Some(lines::DAY_MS),
            )),
            _ => None,
        }
    }
}

fn read_daily(body: &Value) -> Result<Daily, SimpleProviderError> {
    if value::text(body, "/quota").is_some_and(|quota| quota.eq_ignore_ascii_case("unlimited")) {
        return Ok(Daily::Unlimited);
    }
    match (value::number(body, "/quota"), value::number(body, "/used")) {
        (Some(quota), Some(used)) => Ok(Daily::Quota {
            used: used.max(0.0),
            quota: quota.max(0.0),
        }),
        _ => Err(http::decoding(NAME)),
    }
}

/// The balance in dollars, which Chutes reports net of running private instances.
fn read_balance(body: &Value) -> Result<f64, SimpleProviderError> {
    value::number(body, "/balance").ok_or_else(|| http::decoding(NAME))
}

/// The day Chutes counts requests in is the UTC day; its own quota errors name the next 00:00 UTC
/// as the reset.
fn next_utc_midnight(now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let midnight = now.date_naive().succ_opt()?.and_hms_opt(0, 0, 0)?;
    Some(Utc.from_utc_datetime(&midnight))
}

/// Whether paid models refuse requests. Chutes charges a request to the balance once the day's
/// included requests or a subscription cap are used up, and refuses it when the balance is not
/// above zero; free and invoiced accounts are never refused for quota.
fn blocked(subscription: &Subscription, daily: &Daily, balance: f64) -> bool {
    match daily {
        Daily::Unlimited => false,
        Daily::Quota { used, quota } => balance <= 0.0 && (used >= quota || subscription.capped),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Roots;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Duration;
    use serde_json::json;

    const KEY: &str = "cpk_fixture-key";
    const NO_SUBSCRIPTION: &str = r#"{"subscription":false}"#;
    const PRO_DAILY: &str = r#"{"quota":5000,"used":1234.0}"#;
    const RENEWAL: &str = "2026-10-14T08:12:45.123456+00:00";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .unwrap()
    }

    fn renewal() -> DateTime<Utc> {
        at(2026, 10, 14, 8, 12) + Duration::seconds(45) + Duration::microseconds(123_456)
    }

    /// A Pro subscription as Chutes' server computes it: the $20 plan's 4-hour cap is
    /// $20 / 180 × 75 and its monthly cap $20 × 5, both in pay-as-you-go dollars; the 4-hour block
    /// running at 10:00 UTC ends at 12:00.
    fn pro(four_hour_usage: f64, monthly_reset: &str) -> String {
        json!({
            "subscription": true,
            "custom": false,
            "monthly_price": 20.0,
            "anchor_date": "2026-09-14T08:12:45.123456",
            "effective_date": "2026-09-14T08:12:45.123456",
            "updated_at": "2026-09-14T08:12:45.123456",
            "four_hour": {
                "usage": four_hour_usage,
                "cap": 8.333333333333334,
                "remaining": (8.333333333333334 - four_hour_usage).max(0.0),
                "reset_at": "2026-09-27T12:00:00+00:00"
            },
            "monthly": {"usage": 42.0, "cap": 100.0, "remaining": 58.0, "reset_at": monthly_reset}
        })
        .to_string()
    }

    /// `GET /users/me` as Chutes answers it (its `SelfResponse`), with made-up account details.
    fn account(balance: f64) -> String {
        json!({
            "username": "fixture-user",
            "user_id": "00000000-0000-4000-8000-000000000000",
            "logo_id": null,
            "created_at": "2025-08-04T10:00:00",
            "hotkey": "5FixtureHotkey",
            "coldkey": "5FixtureColdkey",
            "payment_address": "5FixturePaymentAddress",
            "permissions_bitmask": 0,
            "balance": balance,
            "netuids": [],
            "quotas": null
        })
        .to_string()
    }

    fn daily_row(used: f64, quota: f64) -> MetricLine {
        lines::count(
            "Daily",
            used,
            quota,
            "requests",
            Some(at(2026, 9, 28, 0, 0)),
            Some(lines::DAY_MS),
        )
    }

    fn labels(reading: &Reading) -> Vec<&str> {
        reading.lines.iter().map(MetricLine::label).collect()
    }

    /// Each of the three requests answered with `(status, body)`.
    async fn run(
        subscription: (u16, &str),
        daily: (u16, &str),
        account: (u16, &str),
    ) -> (Scripted, Result<Reading, SimpleProviderError>) {
        let http = Scripted::new()
            .on("GET", SUBSCRIPTION_URL, subscription.0, subscription.1)
            .on("GET", DAILY_URL, daily.0, daily.1)
            .on("GET", ACCOUNT_URL, account.0, account.1);
        let scope = context_at(&http, json!({ "apiKey": KEY }), now());
        let result = Chutes.fetch(&scope.context()).await;
        (http, result)
    }

    async fn read(subscription: &str, daily: &str, account: &str) -> Reading {
        run((200, subscription), (200, daily), (200, account))
            .await
            .1
            .unwrap()
    }

    #[tokio::test]
    async fn a_pro_subscription_shows_its_caps_daily_requests_balance_and_renewal() {
        let reading = read(&pro(2.5, RENEWAL), PRO_DAILY, &account(7.35)).await;
        assert_eq!(
            reading,
            Reading::new(
                Some("Pro".into()),
                vec![
                    lines::percent(
                        "Session",
                        2.5 / 8.333333333333334 * 100.0,
                        Some(at(2026, 9, 27, 12, 0)),
                        Some(4 * lines::HOUR_MS),
                    ),
                    daily_row(1234.0, 5000.0),
                    lines::percent(
                        "Monthly",
                        42.0 / 100.0 * 100.0,
                        Some(renewal()),
                        Some(lines::MONTH_MS),
                    ),
                    lines::dollar_value("Balance", 7.35),
                ],
            )
            .with_plan_term(Some(PlanTerm::Stated {
                ends_at: renewal(),
                checked_at: None,
            }))
        );
        let declared: Vec<String> = Chutes
            .descriptors(&Provider::new("chutes", "Chutes"))
            .into_iter()
            .map(|descriptor| descriptor.metric_label)
            .collect();
        assert!(
            labels(&reading)
                .iter()
                .all(|label| declared.iter().any(|declared| declared == label))
        );
    }

    #[tokio::test]
    async fn sends_three_bearer_gets_to_the_account_endpoints() {
        let (http, result) = run(
            (200, NO_SUBSCRIPTION),
            (200, r#"{"quota":0,"used":0}"#),
            (200, &account(1.0)),
        )
        .await;
        result.unwrap();
        let mut requests = http.requests();
        requests.sort_by(|left, right| left.url.cmp(&right.url));
        let urls: Vec<&str> = requests
            .iter()
            .map(|request| request.url.as_str())
            .collect();
        assert_eq!(
            urls,
            [
                "https://api.chutes.ai/users/me",
                "https://api.chutes.ai/users/me/quota_usage/me",
                "https://api.chutes.ai/users/me/subscription_usage",
            ]
        );
        for request in &requests {
            assert_eq!(request.method, "GET");
            assert_eq!(
                header(request, "Authorization"),
                Some("Bearer cpk_fixture-key")
            );
            assert_eq!(header(request, "Accept"), Some("application/json"));
            assert_eq!(request.body, None);
            assert_eq!(request.timeout, HttpRequest::DEFAULT_TIMEOUT);
        }
    }

    #[tokio::test]
    async fn a_pay_as_you_go_account_shows_only_its_balance() {
        let reading = read(NO_SUBSCRIPTION, r#"{"quota":0,"used":0.0}"#, &account(12.5)).await;
        assert_eq!(
            reading,
            Reading::new(
                Some("Pay As You Go".into()),
                vec![lines::dollar_value("Balance", 12.5)],
            )
        );
    }

    #[tokio::test]
    async fn a_used_up_daily_quota_with_an_empty_balance_warns_that_paid_models_refuse_requests() {
        let reading = read(
            NO_SUBSCRIPTION,
            r#"{"quota":200,"used":200.0}"#,
            &account(0.0),
        )
        .await;
        assert_eq!(
            reading,
            Reading::new(
                Some("Pay As You Go".into()),
                vec![daily_row(200.0, 200.0), lines::dollar_value("Balance", 0.0)],
            )
            .with_warning(Some(
                "The included usage is used up and the balance is empty, so paid models will \
                 refuse requests."
                    .into()
            ))
        );
        let with_requests_left = read(
            NO_SUBSCRIPTION,
            r#"{"quota":200,"used":37.0}"#,
            &account(0.0),
        )
        .await;
        assert_eq!(with_requests_left.warning, None);
    }

    #[tokio::test]
    async fn a_used_up_cap_warns_only_while_the_balance_is_empty() {
        let capped = pro(8.4, RENEWAL);
        let reading = read(&capped, PRO_DAILY, &account(0.0)).await;
        assert_eq!(reading.warning.as_deref(), Some(BLOCKED));
        let MetricLine::Progress(session) = &reading.lines[0] else {
            panic!("the Session meter")
        };
        assert_eq!((session.label.as_str(), session.used), ("Session", 100.0));
        assert_eq!(read(&capped, PRO_DAILY, &account(3.0)).await.warning, None);
        let owing = read(&capped, PRO_DAILY, &account(-0.42)).await;
        assert_eq!(owing.warning.as_deref(), Some(BLOCKED));
        assert_eq!(
            owing.lines.last(),
            Some(&lines::dollar_value("Balance", 0.0))
        );
    }

    #[tokio::test]
    async fn a_custom_subscription_has_no_monthly_cap_and_renews_a_month_after_its_anchor() {
        let custom = json!({
            "subscription": true,
            "custom": true,
            "monthly_price": 20.0,
            "anchor_date": "2026-08-31T09:30:00",
            "effective_date": "2026-08-31T09:30:00",
            "updated_at": "2026-08-31T09:30:00",
            "four_hour": {
                "usage": 0.0,
                "cap": 8.333333333333334,
                "remaining": 8.333333333333334,
                "reset_at": "2026-09-27T12:00:00+00:00"
            },
            "monthly": {"uncapped": true}
        })
        .to_string();
        let reading = read(&custom, r#"{"quota":5001,"used":12.0}"#, &account(0.0)).await;
        assert_eq!(
            reading,
            Reading::new(
                Some("Custom".into()),
                vec![
                    lines::percent(
                        "Session",
                        0.0,
                        Some(at(2026, 9, 27, 12, 0)),
                        Some(4 * lines::HOUR_MS),
                    ),
                    daily_row(12.0, 5001.0),
                    lines::dollar_value("Balance", 0.0),
                ],
            )
            .with_plan_term(Some(PlanTerm::Stated {
                ends_at: at(2026, 9, 30, 9, 30),
                checked_at: None,
            }))
        );
    }

    #[tokio::test]
    async fn free_and_invoiced_accounts_have_no_daily_limit_and_are_never_warned() {
        let reading = read(
            NO_SUBSCRIPTION,
            r#"{"quota":"unlimited","used":0}"#,
            &account(0.0),
        )
        .await;
        assert_eq!(
            reading,
            Reading::new(None, vec![lines::dollar_value("Balance", 0.0)])
        );
    }

    #[tokio::test]
    async fn a_renewal_that_already_passed_is_left_out() {
        let reading = read(
            &pro(2.5, "2026-09-14T08:12:45+00:00"),
            PRO_DAILY,
            &account(1.0),
        )
        .await;
        let MetricLine::Progress(monthly) = &reading.lines[2] else {
            panic!("the Monthly meter")
        };
        assert_eq!(
            (monthly.label.as_str(), monthly.resets_at),
            ("Monthly", None)
        );
        assert_eq!(reading.plan_term, None);
    }

    #[tokio::test]
    async fn a_part_that_fails_leaves_out_only_its_rows_and_says_so() {
        let (_, result) = run(
            (500, "upstream error"),
            (200, PRO_DAILY),
            (200, &account(7.35)),
        )
        .await;
        assert_eq!(
            result.unwrap(),
            Reading::new(
                None,
                vec![
                    daily_row(1234.0, 5000.0),
                    lines::dollar_value("Balance", 7.35)
                ],
            )
            .with_warning(Some(
                "Could not read the Chutes subscription usage this time.".into()
            ))
        );

        let (_, result) = run(
            (200, r#"{"four_hour":{"usage":1,"cap":2}}"#),
            (200, PRO_DAILY),
            (200, &account(7.35)),
        )
        .await;
        let reading = result.unwrap();
        assert_eq!(labels(&reading), ["Daily", "Balance"]);
        assert_eq!(
            reading.warning.as_deref(),
            Some("Could not read the Chutes subscription usage this time.")
        );

        let (_, result) = run(
            (200, &pro(2.5, RENEWAL)),
            (502, "<html>bad gateway</html>"),
            (200, r#"{"username":"fixture-user"}"#),
        )
        .await;
        let reading = result.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(labels(&reading), ["Session", "Monthly"]);
        assert_eq!(
            reading.warning.as_deref(),
            Some("Could not read the Chutes daily quota and balance this time.")
        );
    }

    #[tokio::test]
    async fn nothing_readable_reports_the_first_failure() {
        let (_, result) = run((503, "busy"), (503, "busy"), (503, "busy")).await;
        let error = result.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Http5xx);
        assert_eq!(error.message, "Chutes answered with HTTP 503.");

        let (_, result) = run((200, "[]"), (200, "{}"), (200, "{}")).await;
        let error = result.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Decoding);
        assert_eq!(
            error.message,
            "Chutes returned usage data this version cannot read."
        );
    }

    #[tokio::test]
    async fn a_refused_key_fails_the_card_without_revealing_the_key() {
        let detail = r#"{"detail":"Invalid token or user not found"}"#;
        let (_, result) = run((401, detail), (401, detail), (401, detail)).await;
        let error = result.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(
            error.message,
            "Chutes refused this API key. Check it, or create a key with account access."
        );
        assert!(!error.message.contains(KEY));

        let (_, result) = run(
            (200, NO_SUBSCRIPTION),
            (200, r#"{"quota":0,"used":0}"#),
            (403, detail),
        )
        .await;
        assert_eq!(result.unwrap_err().category, ErrorCategory::AuthInvalid);
    }

    #[tokio::test]
    async fn a_rate_limit_on_any_request_fails_the_card_so_refreshing_pauses() {
        let (_, result) = run(
            (200, &pro(2.5, RENEWAL)),
            (429, r#"{"detail":"Too many requests"}"#),
            (200, &account(1.0)),
        )
        .await;
        let error = result.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
        assert_eq!(
            error.message,
            "Chutes is rate limiting usage requests. Waiting before retrying."
        );
    }

    #[tokio::test]
    async fn a_secret_without_a_key_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), now());
        let error = Chutes.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(
            error.message,
            "The Chutes API key is missing. Add it again in Accounts."
        );
        assert!(http.requests().is_empty());
    }

    #[test]
    fn connects_with_an_api_key_only_and_links_the_status_usage_and_billing_pages() {
        let connection = Chutes.connection();
        assert_eq!(connection.login_from, None);
        let help = connection.api_key.expect("an API key connection");
        assert_eq!(help.env, ["CHUTES_API_KEY"]);
        assert_eq!(help.url, "https://chutes.ai/app/api");
        assert!(help.fields.is_empty());
        let dir = tempfile::tempdir().unwrap();
        assert!(Chutes.discover(&Roots::under(dir.path())).is_empty());
        let links: Vec<(String, String)> = Chutes
            .links()
            .into_iter()
            .map(|link| (link.label, link.url))
            .collect();
        assert_eq!(
            links,
            [
                ("Status".into(), "https://status.chutes.ai".into()),
                (
                    "Usage".into(),
                    "https://chutes.ai/app/settings/usage".into()
                ),
                (
                    "Billing".into(),
                    "https://chutes.ai/app/settings/billing".into()
                ),
            ]
        );
    }

    #[test]
    fn the_widgets_read_the_card_rows_and_export_their_limits() {
        let provider = Provider::new("chutes@abc", "Chutes");
        let descriptors = Chutes.descriptors(&provider);
        let widgets: Vec<_> = descriptors
            .iter()
            .map(|descriptor| {
                (
                    descriptor.id.as_str(),
                    descriptor.metric_label.as_str(),
                    descriptor.template.kind,
                    descriptor.template.limit,
                )
            })
            .collect();
        assert_eq!(
            widgets,
            [
                (
                    "chutes@abc.session",
                    "Session",
                    MetricKind::Percent,
                    Some(100.0)
                ),
                ("chutes@abc.daily", "Daily", MetricKind::Count, Some(5000.0)),
                (
                    "chutes@abc.monthly",
                    "Monthly",
                    MetricKind::Percent,
                    Some(100.0)
                ),
                ("chutes@abc.balance", "Balance", MetricKind::Dollars, None),
            ]
        );
        assert_eq!(
            descriptors[1].template.count_suffix.as_deref(),
            Some("requests")
        );
        assert_eq!(
            descriptors[1].template.period_duration_ms,
            Some(lines::DAY_MS)
        );
        assert_eq!(
            descriptors[3].template.unbounded_value_word.as_deref(),
            Some("left")
        );
        let exports: Vec<_> = descriptors
            .iter()
            .flat_map(|descriptor| &descriptor.limit_resources)
            .map(|resource| {
                (
                    resource.key.as_str(),
                    resource.unit.as_str(),
                    resource.kind,
                    &resource.source,
                )
            })
            .collect();
        assert_eq!(
            exports,
            [
                (
                    "session",
                    "percent",
                    LimitResourceKind::Consumption,
                    &LimitResourceSource::Progress
                ),
                (
                    "daily",
                    "requests",
                    LimitResourceKind::Consumption,
                    &LimitResourceSource::Progress
                ),
                (
                    "monthly",
                    "percent",
                    LimitResourceKind::Consumption,
                    &LimitResourceSource::Progress
                ),
                (
                    "balance",
                    "usd",
                    LimitResourceKind::Balance,
                    &LimitResourceSource::Value {
                        kind: MetricKind::Dollars,
                        label: None
                    }
                ),
            ]
        );
    }
}
