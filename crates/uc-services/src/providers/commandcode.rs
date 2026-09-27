//! Command Code: the credits, rolling limits and plan behind a Command Code API key (the `user_…`
//! key the `cmd` CLI signs in with, shown on commandcode.ai/studio).
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `COMMANDCODE_API_KEY` environment variable or from a key saved in Quota Control. A refresh calls
//! the CLI's undocumented `alpha` API on `https://api.commandcode.ai`, with the key as a bearer
//! token:
//! - `GET /alpha/whoami?limits=1` for the organization the key belongs to, looked up twice a day;
//! - `GET /alpha/billing/credits?orgId=<org>` for the credits left (the monthly grant, top-ups and
//!   free credits) and the rolling 5-hour and weekly windows (`windowLimits`: `used` of `cap`
//!   credits, `resetAt`), read at every refresh;
//! - `GET /alpha/billing/subscriptions?orgId=<org>` for the plan and the end of its billing period,
//!   looked up every 6 hours and never required.
//!
//! The monthly grant's size is the credits' `monthlyCreditsGranted`, else the allowance
//! commandcode.ai/pricing lists for the plan (checked on 2026-09-27).

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uc_core::{
    ErrorCategory, HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind, MetricLine,
    Provider, ProviderLink, SessionStartSignal, SimpleProviderError, WidgetDescriptor,
};
use url::form_urlencoded;

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct CommandCode;

const NAME: &str = "Command Code";
const WHOAMI_URL: &str = "https://api.commandcode.ai/alpha/whoami?limits=1";
const CREDITS_URL: &str = "https://api.commandcode.ai/alpha/billing/credits";
const SUBSCRIPTIONS_URL: &str = "https://api.commandcode.ai/alpha/billing/subscriptions";
const SESSION: &str = "Session";
const WEEKLY: &str = "Weekly";
const MONTHLY: &str = "Monthly";
const CREDITS: &str = "Credits";
const ORG_MEMO: &str = "commandcode.org";
const PLAN_MEMO: &str = "commandcode.plan";
const NO_KEY: &str = "The Command Code API key is missing. Add it again in Accounts.";
const REFUSED: &str =
    "Command Code refused this API key. Check it or copy a new one from commandcode.ai/studio.";
const PLAN_UNREAD: &str = "Couldn't read your Command Code plan, so its monthly allowance may be missing. Usage below is still up to date.";
/// The credits fields that add up to what the account can still spend: the monthly grant left,
/// top-ups (which roll over) and free credits. `premiumMonthlyCredits` and
/// `opensourceMonthlyCredits` split the monthly grant by model family, so they are not added.
const SPENDABLE: [&str; 3] = ["/monthlyCredits", "/purchasedCredits", "/freeCredits"];

/// Plan ids, their names on commandcode.ai/pricing and their monthly credits in dollars. The
/// Provider plan pays as it goes and Teams pools credits of no listed size, so neither has a
/// fixed grant; `individual-pro` is the Pro plan from before its $80 repricing (`-v1`).
const PLANS: [(&str, &str, Option<f64>); 8] = [
    ("individual-go", "Go", Some(10.0)),
    ("individual-goat", "GOAT", Some(70.0)),
    ("individual-pro", "Pro", Some(30.0)),
    ("individual-pro-v1", "Pro", Some(80.0)),
    ("individual-max", "Max 10×", Some(150.0)),
    ("individual-ultra", "Max 20×", Some(300.0)),
    ("individual-provider", "Provider", None),
    ("teams-pro", "Teams Pro", None),
];

#[async_trait]
impl Service for CommandCode {
    fn id(&self) -> &'static str {
        "commandcode"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Dashboard", "https://commandcode.ai/studio"),
            ProviderLink::new("Billing", "https://commandcode.ai/settings/billing"),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["COMMANDCODE_API_KEY"],
            url: "https://commandcode.ai/studio",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let id = |suffix: &str| format!("{}.{suffix}", provider.id);
        vec![
            // A window opens on the account's first request, so one that has not started yet
            // states no reset.
            WidgetDescriptor::percent(
                id("session"),
                provider,
                SESSION,
                None,
                Some(SessionStartSignal::MissingResetDate),
            )
            .exporting_progress("session", "percent"),
            WidgetDescriptor::percent(id("weekly"), provider, WEEKLY, None, None)
                .exporting_progress("weekly", "percent"),
            WidgetDescriptor::bounded_dollars(
                id("monthly"),
                provider,
                MONTHLY,
                None,
                100.0,
                None,
                Some("spent"),
            )
            .exporting_progress("monthly", "usd"),
            WidgetDescriptor::dollar_balance(id("credits"), provider, CREDITS, None, "left")
                .exporting_limit(
                    "credits",
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
        let key = context.secret.key().ok_or_else(|| http::invalid(NO_KEY))?;
        let result = read(context, key).await;
        if matches!(&result, Err(error) if error.category == ErrorCategory::AuthInvalid) {
            context.memo.remove(ORG_MEMO).await;
            context.memo.remove(PLAN_MEMO).await;
        }
        result
    }
}

async fn read(context: &FetchContext<'_>, key: &str) -> Result<Reading, SimpleProviderError> {
    let print = fingerprint(key);
    let (org, body) = org_and_credits(context, key, &print).await?;
    let credits = parse_credits(&body)?;
    let lookup = subscription(context, key, &print, org.as_deref()).await;
    let (plan, listed_grant, period_end) = match &lookup {
        Some(Subscription::Free) => (Some("Free".to_string()), None, None),
        Some(Subscription::Paid {
            plan_id,
            period_end,
        }) => (plan_display(plan_id), plan_grant(plan_id), *period_end),
        None => (None, None, None),
    };
    let mut meters = credits.windows;
    if let (Some(total), Some(left)) = (credits.granted.or(listed_grant), credits.monthly_left) {
        // Without the billing period's end, the row states no cadence either: "resets in 30
        // days" would be wrong for most of the month.
        let resets_at = period_end.filter(|end| *end > context.now);
        meters.push(lines::dollars(
            MONTHLY,
            (total - left).clamp(0.0, total),
            total,
            resets_at,
            resets_at.map(|_| lines::MONTH_MS),
        ));
    }
    meters.push(lines::dollar_value(CREDITS, credits.spendable));
    let warning = lookup.is_none().then(|| PLAN_UNREAD.to_string());
    Ok(Reading::new(plan, meters).with_warning(warning))
}

/// The key's organization and its credits. When the credits call refuses a remembered
/// organization, the organization is looked up once more, in case the key moved to another one.
async fn org_and_credits(
    context: &FetchContext<'_>,
    key: &str,
    print: &str,
) -> Result<(Option<String>, Value), SimpleProviderError> {
    if let Some(org) = remembered_org(context, print).await {
        match credits(context, key, org.as_deref()).await {
            Err(error) if error.category == ErrorCategory::AuthInvalid => {}
            result => return result.map(|credits| (org, credits)),
        }
    }
    let org = whoami(context, key, print).await?;
    let credits = credits(context, key, org.as_deref()).await?;
    Ok((org, credits))
}

/// The remembered organization of this key: `None` when nothing is remembered for it, `Some(None)`
/// when the key belongs to no organization.
async fn remembered_org(context: &FetchContext<'_>, print: &str) -> Option<Option<String>> {
    let memo = context.memo.get(ORG_MEMO, context.now).await?;
    (memo["key"].as_str() == Some(print)).then(|| memo["org"].as_str().map(str::to_string))
}

/// The organization the key belongs to, remembered for 12 hours. An answer without one leaves the
/// organization out of the billing requests, as the Command Code CLI does.
async fn whoami(
    context: &FetchContext<'_>,
    key: &str,
    print: &str,
) -> Result<Option<String>, SimpleProviderError> {
    let body = get_json(context, WHOAMI_URL, key).await?;
    let org = match body.pointer("/org/id") {
        Some(Value::String(id)) => Some(id.trim().to_string()).filter(|id| !id.is_empty()),
        Some(Value::Number(id)) => Some(id.to_string()),
        _ => None,
    };
    context
        .memo
        .put(
            ORG_MEMO,
            json!({ "key": print, "org": org }),
            Some(context.now + Duration::hours(12)),
        )
        .await;
    Ok(org)
}

async fn credits(
    context: &FetchContext<'_>,
    key: &str,
    org: Option<&str>,
) -> Result<Value, SimpleProviderError> {
    get_json(context, &with_org(CREDITS_URL, org), key).await
}

/// What the subscriptions endpoint says about the account's plan.
#[derive(Clone, Debug, PartialEq)]
enum Subscription {
    /// A successful answer with `data: null`: the account has no paid plan.
    Free,
    Paid {
        plan_id: String,
        period_end: Option<DateTime<Utc>>,
    },
}

/// The plan, remembered for 6 hours but never past the end of its billing period. `None` when the
/// lookup failed; a failure is not remembered, so the next refresh asks again.
async fn subscription(
    context: &FetchContext<'_>,
    key: &str,
    print: &str,
    org: Option<&str>,
) -> Option<Subscription> {
    if let Some(memo) = context.memo.get(PLAN_MEMO, context.now).await
        && memo["key"].as_str() == Some(print)
        && memo["org"].as_str() == org
    {
        return Some(match memo["plan"].as_str() {
            Some(plan_id) => Subscription::Paid {
                plan_id: plan_id.to_string(),
                period_end: value::time(&memo, "/periodEnd"),
            },
            None => Subscription::Free,
        });
    }
    let response = http::send(
        context.http,
        request(&with_org(SUBSCRIPTIONS_URL, org), key),
        NAME,
    )
    .await
    .ok()?;
    if !response.is_success() {
        return None;
    }
    let found = parse_subscription(&http::parse(&response, NAME).ok()?)?;
    let mut expires = context.now + Duration::hours(6);
    let (plan_id, period_end) = match &found {
        Subscription::Free => (None, None),
        Subscription::Paid {
            plan_id,
            period_end,
        } => {
            if let Some(end) = period_end.filter(|end| *end > context.now) {
                expires = expires.min(end);
            }
            (
                Some(plan_id.as_str()),
                period_end.map(|end| end.to_rfc3339()),
            )
        }
    };
    context
        .memo
        .put(
            PLAN_MEMO,
            json!({ "key": print, "org": org, "plan": plan_id, "periodEnd": period_end }),
            Some(expires),
        )
        .await;
    Some(found)
}

/// A subscriptions answer: its plan, or the free tier when it states `data: null`. `None` for a
/// failure envelope (`success: false`) or an answer this version cannot read.
fn parse_subscription(body: &Value) -> Option<Subscription> {
    if value::flag(body, "/success") == Some(false) {
        return None;
    }
    match body.get("data")? {
        Value::Null => Some(Subscription::Free),
        data => Some(Subscription::Paid {
            plan_id: value::text(data, "/planId")?.to_string(),
            period_end: value::time(data, "/currentPeriodEnd"),
        }),
    }
}

fn plan_entry(plan_id: &str) -> Option<&'static (&'static str, &'static str, Option<f64>)> {
    PLANS
        .iter()
        .find(|(id, _, _)| id.eq_ignore_ascii_case(plan_id))
}

/// The plan's name: the one commandcode.ai lists, else the id without its `individual-` prefix
/// (`individual-starter` → `Starter`, `teams-max` → `Teams Max`).
fn plan_display(plan_id: &str) -> Option<String> {
    if let Some((_, name, _)) = plan_entry(plan_id) {
        return Some(name.to_string());
    }
    let id = plan_id.to_ascii_lowercase();
    lines::plan_name(id.strip_prefix("individual-").unwrap_or(&id))
}

fn plan_grant(plan_id: &str) -> Option<f64> {
    plan_entry(plan_id).and_then(|(_, _, grant)| *grant)
}

/// What a credits answer says.
struct Credits {
    /// The Session and Weekly meters, for the windows the answer carries.
    windows: Vec<MetricLine>,
    /// The monthly grant left (`monthlyCredits`), in dollars.
    monthly_left: Option<f64>,
    /// The monthly grant's size, when the answer states it (`monthlyCreditsGranted`).
    granted: Option<f64>,
    /// Everything the account can still spend, in dollars.
    spendable: f64,
}

/// A credits answer read. One without a `credits` object, or without any spendable amount in it,
/// is a decoding error rather than a zero balance. The rolling windows sit at the root of the
/// answer or inside `credits`.
fn parse_credits(body: &Value) -> Result<Credits, SimpleProviderError> {
    let credits = body
        .get("credits")
        .filter(|credits| credits.is_object())
        .ok_or_else(|| http::decoding(NAME))?;
    let amounts: Vec<f64> = SPENDABLE
        .iter()
        .filter_map(|pointer| value::number(credits, pointer))
        .collect();
    if amounts.is_empty() {
        return Err(http::decoding(NAME));
    }
    let limits = body
        .get("windowLimits")
        .or_else(|| credits.get("windowLimits"));
    Ok(Credits {
        windows: [
            window(limits, "fiveHour", SESSION, 5 * lines::HOUR_MS),
            window(limits, "weekly", WEEKLY, lines::WEEK_MS),
        ]
        .into_iter()
        .flatten()
        .collect(),
        monthly_left: value::number(credits, "/monthlyCredits"),
        granted: value::number(credits, "/monthlyCreditsGranted").filter(|granted| *granted > 0.0),
        spendable: amounts.iter().sum::<f64>().max(0.0),
    })
}

/// A rolling window's used share: `used` of `cap` credits, or full once Command Code says the
/// window is `exceeded`. `None` when the window is missing or has no cap.
fn window(limits: Option<&Value>, name: &str, label: &str, period_ms: i64) -> Option<MetricLine> {
    let window = limits?.get(name)?;
    let cap = value::number(window, "/cap").filter(|cap| *cap > 0.0)?;
    let used = if value::flag(window, "/exceeded") == Some(true) {
        100.0
    } else {
        value::number(window, "/used").unwrap_or(0.0) / cap * 100.0
    };
    Some(lines::percent(
        label,
        used,
        value::time(window, "/resetAt"),
        Some(period_ms),
    ))
}

/// A Command Code API answer as JSON. A refused key (401 or 403) becomes `REFUSED`, any other
/// failure its status.
async fn get_json(
    context: &FetchContext<'_>,
    url: &str,
    key: &str,
) -> Result<Value, SimpleProviderError> {
    let response = http::send(context.http, request(url, key), NAME).await?;
    if matches!(response.status, 401 | 403) {
        return Err(http::invalid(REFUSED));
    }
    if !response.is_success() {
        return Err(http::status_error(&response, NAME));
    }
    http::parse(&response, NAME)
}

fn request(url: &str, key: &str) -> HttpRequest {
    HttpRequest::get(url)
        .bearer(key)
        .header("Accept", "application/json")
}

/// `url` with the `orgId` query the billing endpoints take, when the key has an organization.
fn with_org(url: &str, org: Option<&str>) -> String {
    match org {
        Some(org) => format!(
            "{url}?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("orgId", org)
                .finish()
        ),
        None => url.to_string(),
    }
}

/// A short digest of the key, so a lookup remembered for one key is never used for another.
fn fingerprint(key: &str) -> String {
    Sha256::digest(key.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Memo, Roots, Secret};
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;

    const KEY: &str = "user_test_0123456789abcdef";
    const WHOAMI: &str = "https://api.commandcode.ai/alpha/whoami";
    const ORG_CREDITS: &str = "https://api.commandcode.ai/alpha/billing/credits?orgId=org_7f3a9c";
    const ORG_SUBSCRIPTIONS: &str =
        "https://api.commandcode.ai/alpha/billing/subscriptions?orgId=org_7f3a9c";

    /// The account behind the key, in the shape 9router's fixture shows (anonymized).
    const WHOAMI_BODY: &str = r#"{"user":{"id":"usr_test","name":"Test User","email":"dev@example.com"},"org":{"id":"org_7f3a9c","name":"personal"}}"#;
    /// A GOAT account: the credits fields CodexBar captured from api.commandcode.ai plus 9router's
    /// `freeCredits`, with GOAT's windows from commandcode.ai/docs ($14 per 5 hours, $35 a week).
    const CREDITS_BODY: &str = r#"{"credits":{"belowThreshold":false,"creditThreshold":0,
        "monthlyCredits":52.5,"purchasedCredits":12,"freeCredits":0.5,"premiumMonthlyCredits":10,
        "opensourceMonthlyCredits":42.5,"monthlyCreditsGranted":70},
        "windowLimits":{
            "fiveHour":{"used":3.5,"cap":14,"resetAt":1790510400000,"exceeded":false},
            "weekly":{"used":17.5,"cap":35,"resetAt":1790848800000,"exceeded":false}}}"#;
    /// An active GOAT subscription, in the shape CodexBar captured (anonymized).
    const SUBSCRIPTION_BODY: &str = r#"{"success":true,"data":{"id":"sub_test","status":"active",
        "orgId":"org_7f3a9c","quantity":1,"cancelAtPeriodEnd":false,
        "currentPeriodStart":"2026-09-06T07:28:50.000Z","currentPeriodEnd":"2026-10-06T07:28:50.000Z",
        "endedAt":null,"cancelAt":null,"canceledAt":null,"planId":"individual-goat"}}"#;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
            .unwrap()
    }

    fn secret() -> Value {
        json!({ "apiKey": KEY })
    }

    fn period_end() -> Option<DateTime<Utc>> {
        Some(at(2026, 10, 6, 7, 28, 50))
    }

    fn session(used: f64) -> MetricLine {
        lines::percent(
            "Session",
            used,
            Some(at(2026, 9, 27, 12, 0, 0)),
            Some(5 * lines::HOUR_MS),
        )
    }

    fn weekly(used: f64) -> MetricLine {
        lines::percent(
            "Weekly",
            used,
            Some(at(2026, 10, 1, 10, 0, 0)),
            Some(lines::WEEK_MS),
        )
    }

    fn monthly(used: f64, total: f64, resets_at: Option<DateTime<Utc>>) -> MetricLine {
        let period = resets_at.map(|_| lines::MONTH_MS);
        lines::dollars("Monthly", used, total, resets_at, period)
    }

    fn credits_row(amount: f64) -> MetricLine {
        lines::dollar_value("Credits", amount)
    }

    fn goat_account(credits: &str, subscription: &str) -> Scripted {
        Scripted::new()
            .on("GET", WHOAMI, 200, WHOAMI_BODY)
            .on("GET", CREDITS_URL, 200, credits)
            .on("GET", SUBSCRIPTIONS_URL, 200, subscription)
    }

    async fn fetch_once(http: &Scripted) -> Result<Reading, SimpleProviderError> {
        let scope = context_at(http, secret(), now());
        CommandCode.fetch(&scope.context()).await
    }

    fn urls(http: &Scripted) -> Vec<String> {
        http.requests()
            .into_iter()
            .map(|request| request.url)
            .collect()
    }

    #[tokio::test]
    async fn reads_the_windows_the_monthly_grant_the_credits_and_the_plan() {
        let http = goat_account(CREDITS_BODY, SUBSCRIPTION_BODY);
        let reading = fetch_once(&http).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("GOAT".into()),
                vec![
                    session(25.0),
                    weekly(50.0),
                    monthly(17.5, 70.0, period_end()),
                    credits_row(65.0),
                ]
            )
        );
        let MetricLine::Progress(month) = &reading.lines[2] else {
            panic!("the Monthly meter")
        };
        assert_eq!(
            (month.used, month.limit, month.format.clone()),
            (17.5, 70.0, uc_core::ProgressFormat::Dollars)
        );
        assert_eq!(month.resets_at, period_end());
        assert_eq!(month.period_duration_ms, Some(30 * 24 * 3_600_000));
        assert_eq!(reading.plan_term, None);
    }

    #[tokio::test]
    async fn sends_the_key_as_a_bearer_token_and_scopes_billing_to_the_organization() {
        let http = goat_account(CREDITS_BODY, SUBSCRIPTION_BODY);
        fetch_once(&http).await.unwrap();
        let requests = http.requests();
        assert_eq!(
            urls(&http),
            [
                "https://api.commandcode.ai/alpha/whoami?limits=1",
                ORG_CREDITS,
                ORG_SUBSCRIPTIONS,
            ]
        );
        for request in &requests {
            assert_eq!(request.method, "GET");
            assert!(!request.url.contains(KEY), "the key stays out of the URL");
            assert_eq!(
                header(request, "Authorization"),
                Some("Bearer user_test_0123456789abcdef")
            );
            assert_eq!(header(request, "Accept"), Some("application/json"));
            assert_eq!(request.body, None);
            assert_eq!(request.timeout, HttpRequest::DEFAULT_TIMEOUT);
        }
    }

    #[tokio::test]
    async fn the_organization_and_plan_are_remembered_between_refreshes() {
        let http = goat_account(CREDITS_BODY, SUBSCRIPTION_BODY);
        let scope = context_at(&http, secret(), now());
        let first = CommandCode.fetch(&scope.context()).await.unwrap();
        let second = CommandCode.fetch(&scope.context()).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(
            urls(&http),
            [
                "https://api.commandcode.ai/alpha/whoami?limits=1",
                ORG_CREDITS,
                ORG_SUBSCRIPTIONS,
                ORG_CREDITS,
            ]
        );
    }

    #[tokio::test]
    async fn the_plan_is_looked_up_again_after_six_hours_or_when_its_period_ends() {
        let ends_at_noon =
            SUBSCRIPTION_BODY.replace("2026-10-06T07:28:50.000Z", "2026-09-27T12:00:00.000Z");
        for (subscription, last_remembered, looked_up_again) in [
            (
                SUBSCRIPTION_BODY.to_string(),
                at(2026, 9, 27, 15, 59, 0),
                at(2026, 9, 27, 16, 0, 0),
            ),
            (
                ends_at_noon,
                at(2026, 9, 27, 11, 59, 0),
                at(2026, 9, 27, 12, 0, 0),
            ),
        ] {
            let http = goat_account(CREDITS_BODY, &subscription);
            let memo = Memo::default();
            let shared = http.shared();
            let secret = Secret::api_key(KEY);
            let lookups = |http: &Scripted| {
                urls(http)
                    .iter()
                    .filter(|url| url.starts_with(SUBSCRIPTIONS_URL))
                    .count()
            };
            for (time, expected) in [(now(), 1), (last_remembered, 1), (looked_up_again, 2)] {
                let context = FetchContext {
                    secret: &secret,
                    http: &shared,
                    now: time,
                    memo: &memo,
                };
                CommandCode.fetch(&context).await.unwrap();
                assert_eq!(lookups(&http), expected, "{subscription} at {time}");
            }
            let whoami_calls = urls(&http)
                .iter()
                .filter(|url| url.starts_with(WHOAMI))
                .count();
            assert_eq!(whoami_calls, 1, "the organization stands for 12 hours");
        }
    }

    #[tokio::test]
    async fn a_lookup_remembered_for_one_key_is_not_used_for_another() {
        let http = Scripted::new()
            .on("GET", WHOAMI, 200, WHOAMI_BODY)
            .on(
                "GET",
                WHOAMI,
                200,
                r#"{"org":{"id":"org_other","name":"acme"}}"#,
            )
            .on("GET", CREDITS_URL, 200, CREDITS_BODY)
            .on("GET", SUBSCRIPTIONS_URL, 200, SUBSCRIPTION_BODY);
        let memo = Memo::default();
        let shared = http.shared();
        let first = Secret::api_key(KEY);
        let replaced = Secret::api_key("user_replaced_key");
        for secret in [&first, &replaced] {
            let context = FetchContext {
                secret,
                http: &shared,
                now: now(),
                memo: &memo,
            };
            CommandCode.fetch(&context).await.unwrap();
        }
        let requests = http.requests();
        assert_eq!(requests.len(), 6);
        assert_eq!(
            requests[3].url,
            "https://api.commandcode.ai/alpha/whoami?limits=1"
        );
        assert_eq!(
            header(&requests[3], "Authorization"),
            Some("Bearer user_replaced_key")
        );
        assert_eq!(
            requests[4].url,
            "https://api.commandcode.ai/alpha/billing/credits?orgId=org_other"
        );
        assert_eq!(
            requests[5].url,
            "https://api.commandcode.ai/alpha/billing/subscriptions?orgId=org_other"
        );
    }

    #[tokio::test]
    async fn without_a_stated_grant_the_monthly_row_is_sized_from_the_plan() {
        let credits = r#"{"credits":{"monthlyCredits":72.5,"purchasedCredits":0},
            "windowLimits":{"fiveHour":{"used":4,"cap":16,"resetAt":1790510400},
            "weekly":{"used":10,"cap":40,"resetAt":"1790848800000"}}}"#;
        let pro = SUBSCRIPTION_BODY.replace("individual-goat", "individual-pro-v1");
        let reading = fetch_once(&goat_account(credits, &pro)).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(
                Some("Pro".into()),
                vec![
                    session(25.0),
                    weekly(25.0),
                    monthly(7.5, 80.0, period_end()),
                    credits_row(72.5),
                ]
            )
        );
    }

    #[tokio::test]
    async fn plans_take_the_names_and_allowances_commandcode_ai_lists() {
        let credits = r#"{"credits":{"monthlyCredits":100,"purchasedCredits":0}}"#;
        let plans: [(&str, &str, Option<f64>); 8] = [
            ("individual-go", "Go", Some(10.0)),
            ("individual-pro", "Pro", Some(30.0)),
            ("individual-max", "Max 10×", Some(150.0)),
            ("INDIVIDUAL-ULTRA", "Max 20×", Some(300.0)),
            ("individual-provider", "Provider", None),
            ("teams-pro", "Teams Pro", None),
            ("individual-starter", "Starter", None),
            ("teams-max", "Teams Max", None),
        ];
        for (plan_id, name, total) in plans {
            let subscription = SUBSCRIPTION_BODY.replace("individual-goat", plan_id);
            let reading = fetch_once(&goat_account(credits, &subscription))
                .await
                .unwrap();
            let mut expected = Vec::new();
            if let Some(total) = total {
                expected.push(monthly((total - 100.0).max(0.0), total, period_end()));
            }
            expected.push(credits_row(100.0));
            assert_eq!(
                reading,
                Reading::new(Some(name.into()), expected),
                "{plan_id}"
            );
        }
    }

    #[tokio::test]
    async fn a_free_account_shows_its_credits_without_a_monthly_grant() {
        let credits = r#"{"credits":{"monthlyCredits":0,"purchasedCredits":5,"freeCredits":1.25}}"#;
        let http = goat_account(credits, r#"{"success":true,"data":null}"#);
        let scope = context_at(&http, secret(), now());
        let reading = CommandCode.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading,
            Reading::new(Some("Free".into()), vec![credits_row(6.25)])
        );
        CommandCode.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            urls(&http)
                .iter()
                .filter(|url| url.starts_with(SUBSCRIPTIONS_URL))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn a_failed_plan_lookup_keeps_the_usage_warns_and_is_retried() {
        let http = Scripted::new()
            .on("GET", WHOAMI, 200, WHOAMI_BODY)
            .on("GET", CREDITS_URL, 200, CREDITS_BODY)
            .on("GET", SUBSCRIPTIONS_URL, 503, "busy")
            .on(
                "GET",
                SUBSCRIPTIONS_URL,
                200,
                r#"{"success":false,"error":"temporarily unavailable"}"#,
            )
            .on("GET", SUBSCRIPTIONS_URL, 200, SUBSCRIPTION_BODY);
        let scope = context_at(&http, secret(), now());
        let warned = Reading::new(
            None,
            vec![
                session(25.0),
                weekly(50.0),
                monthly(17.5, 70.0, None),
                credits_row(65.0),
            ],
        )
        .with_warning(Some(PLAN_UNREAD.into()));
        let first = CommandCode.fetch(&scope.context()).await.unwrap();
        assert_eq!(first, warned);
        let MetricLine::Progress(month) = &first.lines[2] else {
            panic!("the Monthly meter")
        };
        assert_eq!(
            (month.resets_at, month.period_duration_ms),
            (None, None),
            "no billing period, so no reset and no cadence"
        );
        assert_eq!(CommandCode.fetch(&scope.context()).await.unwrap(), warned);
        let third = CommandCode.fetch(&scope.context()).await.unwrap();
        assert_eq!(third.plan.as_deref(), Some("GOAT"));
        assert_eq!(third.warning, None);
        assert_eq!(
            PLAN_UNREAD,
            "Couldn't read your Command Code plan, so its monthly allowance may be missing. Usage below is still up to date."
        );
    }

    #[tokio::test]
    async fn a_failed_plan_lookup_without_a_stated_grant_leaves_the_monthly_row_out() {
        let credits = r#"{"credits":{"monthlyCredits":8.7784,"purchasedCredits":0,
            "premiumMonthlyCredits":0,"opensourceMonthlyCredits":8.7784}}"#;
        let http = Scripted::new()
            .on("GET", WHOAMI, 200, WHOAMI_BODY)
            .on("GET", CREDITS_URL, 200, credits)
            .on("GET", SUBSCRIPTIONS_URL, 200, r#"{"success":true}"#);
        let reading = fetch_once(&http).await.unwrap();
        assert_eq!(reading.plan, None);
        assert_eq!(reading.lines, vec![credits_row(8.7784)]);
        assert_eq!(reading.warning.as_deref(), Some(PLAN_UNREAD));
    }

    #[tokio::test]
    async fn windows_nested_in_credits_with_text_numbers_are_read() {
        let credits = r#"{"credits":{"monthlyCredits":7.25,"purchasedCredits":2,
            "windowLimits":{
                "fiveHour":{"cap":"4","used":"1","resetAt":"1790510400"},
                "weekly":{"cap":20,"used":4,"resetAt":1790848800000}}}}"#;
        let go = SUBSCRIPTION_BODY.replace("individual-goat", "individual-go");
        let reading = fetch_once(&goat_account(credits, &go)).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![
                session(25.0),
                weekly(20.0),
                monthly(2.75, 10.0, period_end()),
                credits_row(9.25),
            ]
        );
    }

    #[tokio::test]
    async fn an_unopened_window_states_no_reset_and_an_exceeded_one_is_full() {
        let credits = r#"{"credits":{"monthlyCredits":1,"purchasedCredits":0},
            "windowLimits":{
                "fiveHour":{"used":0,"cap":14,"resetAt":0,"exceeded":false},
                "weekly":{"used":30,"cap":35,"resetAt":1790848800000,"exceeded":true}}}"#;
        let reading = fetch_once(&goat_account(credits, SUBSCRIPTION_BODY))
            .await
            .unwrap();
        assert_eq!(
            reading.lines[..2],
            [
                lines::percent("Session", 0.0, None, Some(5 * lines::HOUR_MS)),
                weekly(100.0),
            ]
        );
    }

    #[tokio::test]
    async fn windows_without_a_cap_are_left_out() {
        let credits = r#"{"credits":{"monthlyCredits":70,"purchasedCredits":0},
            "windowLimits":{"fiveHour":{"used":0,"cap":0},"weekly":null}}"#;
        let reading = fetch_once(&goat_account(credits, SUBSCRIPTION_BODY))
            .await
            .unwrap();
        assert_eq!(
            reading.lines,
            vec![monthly(0.0, 70.0, period_end()), credits_row(70.0)]
        );
    }

    #[tokio::test]
    async fn a_key_without_an_organization_asks_for_billing_without_one() {
        let http = Scripted::new()
            .on(
                "GET",
                WHOAMI,
                200,
                r#"{"user":{"id":"usr_test"},"org":null}"#,
            )
            .on("GET", CREDITS_URL, 200, CREDITS_BODY)
            .on("GET", SUBSCRIPTIONS_URL, 200, SUBSCRIPTION_BODY);
        let scope = context_at(&http, secret(), now());
        CommandCode.fetch(&scope.context()).await.unwrap();
        CommandCode.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            urls(&http),
            [
                "https://api.commandcode.ai/alpha/whoami?limits=1",
                "https://api.commandcode.ai/alpha/billing/credits",
                "https://api.commandcode.ai/alpha/billing/subscriptions",
                "https://api.commandcode.ai/alpha/billing/credits",
            ]
        );
    }

    #[tokio::test]
    async fn an_organization_the_credits_call_refuses_is_looked_up_again_once() {
        let http = Scripted::new()
            .on("GET", WHOAMI, 200, WHOAMI_BODY)
            .on(
                "GET",
                WHOAMI,
                200,
                r#"{"org":{"id":"org_moved","name":"acme"}}"#,
            )
            .on("GET", CREDITS_URL, 200, CREDITS_BODY)
            .on("GET", CREDITS_URL, 403, r#"{"error":"forbidden"}"#)
            .on("GET", CREDITS_URL, 200, CREDITS_BODY)
            .on("GET", SUBSCRIPTIONS_URL, 200, SUBSCRIPTION_BODY);
        let scope = context_at(&http, secret(), now());
        CommandCode.fetch(&scope.context()).await.unwrap();
        let reading = CommandCode.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("GOAT"));
        assert_eq!(
            urls(&http)[3..],
            [
                ORG_CREDITS.to_string(),
                "https://api.commandcode.ai/alpha/whoami?limits=1".to_string(),
                "https://api.commandcode.ai/alpha/billing/credits?orgId=org_moved".to_string(),
                "https://api.commandcode.ai/alpha/billing/subscriptions?orgId=org_moved"
                    .to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn a_refused_key_is_reported_without_revealing_it_and_forgotten() {
        for status in [401, 403] {
            let http = Scripted::new().on(
                "GET",
                WHOAMI,
                status,
                r#"{"error":"Invalid API key user_test_0123456789abcdef"}"#,
            );
            let error = fetch_once(&http).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::AuthInvalid, "{status}");
            assert_eq!(
                error.message,
                "Command Code refused this API key. Check it or copy a new one from commandcode.ai/studio."
            );
            assert!(!error.message.contains(KEY));
            assert_eq!(http.requests().len(), 1);
        }

        let http = Scripted::new()
            .on("GET", WHOAMI, 200, WHOAMI_BODY)
            .on("GET", CREDITS_URL, 200, CREDITS_BODY)
            .on("GET", CREDITS_URL, 401, "{}")
            .on("GET", SUBSCRIPTIONS_URL, 200, SUBSCRIPTION_BODY);
        let scope = context_at(&http, secret(), now());
        CommandCode.fetch(&scope.context()).await.unwrap();
        let error = CommandCode.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(
            scope.context().memo.get(ORG_MEMO, now()).await,
            None,
            "a refused key forgets its organization"
        );
        assert_eq!(scope.context().memo.get(PLAN_MEMO, now()).await, None);
    }

    #[tokio::test]
    async fn rate_limits_and_server_errors_keep_their_categories() {
        let limited = Scripted::new().on("GET", WHOAMI, 200, WHOAMI_BODY).on(
            "GET",
            CREDITS_URL,
            429,
            r#"{"error":"Too many requests"}"#,
        );
        let error = fetch_once(&limited).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
        assert_eq!(
            error.message,
            "Command Code is rate limiting usage requests. Waiting before retrying."
        );

        let whoami_limited = Scripted::new().on("GET", WHOAMI, 429, "{}");
        assert_eq!(
            fetch_once(&whoami_limited).await.unwrap_err().category,
            ErrorCategory::RateLimited
        );

        let down = Scripted::new().on("GET", WHOAMI, 200, WHOAMI_BODY).on(
            "GET",
            CREDITS_URL,
            503,
            "Service Unavailable",
        );
        let error = fetch_once(&down).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Http5xx);
        assert_eq!(error.message, "Command Code answered with HTTP 503.");
    }

    #[tokio::test]
    async fn credits_this_version_cannot_read_are_a_decoding_error_asked_about_no_further() {
        for body in [
            "<html>busy</html>",
            r#"{"windowLimits":{}}"#,
            r#"{"credits":null}"#,
            r#"{"credits":{"belowThreshold":false,"creditThreshold":0}}"#,
        ] {
            let http = goat_account(body, SUBSCRIPTION_BODY);
            let error = fetch_once(&http).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::Decoding, "{body}");
            assert_eq!(
                error.message,
                "Command Code returned usage data this version cannot read."
            );
            assert_eq!(
                urls(&http),
                [
                    "https://api.commandcode.ai/alpha/whoami?limits=1",
                    ORG_CREDITS
                ],
                "{body}"
            );
        }
    }

    #[tokio::test]
    async fn a_secret_without_a_key_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), now());
        let error = CommandCode.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(
            error.message,
            "The Command Code API key is missing. Add it again in Accounts."
        );
        assert!(http.requests().is_empty());
    }

    #[test]
    fn subscription_answers_are_read_like_commandcode_ai_writes_them() {
        assert_eq!(
            parse_subscription(&serde_json::from_str(SUBSCRIPTION_BODY).unwrap()),
            Some(Subscription::Paid {
                plan_id: "individual-goat".into(),
                period_end: period_end(),
            })
        );
        assert_eq!(
            parse_subscription(&json!({ "data": { "planId": "individual-go" } })),
            Some(Subscription::Paid {
                plan_id: "individual-go".into(),
                period_end: None,
            })
        );
        assert_eq!(
            parse_subscription(&json!({ "success": true, "data": null })),
            Some(Subscription::Free)
        );
        for failed in [
            json!({ "success": false, "data": null }),
            json!({ "success": true }),
            json!({ "data": { "status": "active" } }),
        ] {
            assert_eq!(parse_subscription(&failed), None, "{failed}");
        }
    }

    #[test]
    fn connects_with_an_api_key_only() {
        let connection = CommandCode.connection();
        assert_eq!(connection.login_from, None);
        let help = connection.api_key.expect("an API key connection");
        assert_eq!(help.env, ["COMMANDCODE_API_KEY"]);
        assert_eq!(help.url, "https://commandcode.ai/studio");
        assert!(help.fields.is_empty());
        let dir = tempfile::tempdir().unwrap();
        assert!(CommandCode.discover(&Roots::under(dir.path())).is_empty());
        assert_eq!(CommandCode.id(), "commandcode");
        assert_eq!(CommandCode.name(), "Command Code");
        let links: Vec<_> = CommandCode
            .links()
            .into_iter()
            .map(|link| (link.label, link.url))
            .collect();
        assert_eq!(
            links,
            [
                (
                    "Dashboard".to_string(),
                    "https://commandcode.ai/studio".to_string()
                ),
                (
                    "Billing".to_string(),
                    "https://commandcode.ai/settings/billing".to_string()
                ),
            ]
        );
    }

    #[test]
    fn the_widgets_read_the_rows_the_reading_writes_and_export_them() {
        let provider = Provider::new("commandcode@abc", "Command Code");
        let descriptors = CommandCode.descriptors(&provider);
        let summary: Vec<_> = descriptors
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
            summary,
            [
                (
                    "commandcode@abc.session",
                    "Session",
                    MetricKind::Percent,
                    Some(100.0)
                ),
                (
                    "commandcode@abc.weekly",
                    "Weekly",
                    MetricKind::Percent,
                    Some(100.0)
                ),
                (
                    "commandcode@abc.monthly",
                    "Monthly",
                    MetricKind::Dollars,
                    Some(100.0)
                ),
                (
                    "commandcode@abc.credits",
                    "Credits",
                    MetricKind::Dollars,
                    None
                ),
            ]
        );
        assert_eq!(
            descriptors[0].template.session_start_signal,
            Some(SessionStartSignal::MissingResetDate)
        );
        assert_eq!(descriptors[1].template.session_start_signal, None);
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
                    resource.kind,
                    resource.unit.as_str(),
                    &resource.source,
                )
            })
            .collect();
        assert_eq!(
            exports,
            [
                (
                    "session",
                    LimitResourceKind::Consumption,
                    "percent",
                    &LimitResourceSource::Progress
                ),
                (
                    "weekly",
                    LimitResourceKind::Consumption,
                    "percent",
                    &LimitResourceSource::Progress
                ),
                (
                    "monthly",
                    LimitResourceKind::Consumption,
                    "usd",
                    &LimitResourceSource::Progress
                ),
                (
                    "credits",
                    LimitResourceKind::Balance,
                    "usd",
                    &LimitResourceSource::Value {
                        kind: MetricKind::Dollars,
                        label: None
                    }
                ),
            ]
        );
    }
}
